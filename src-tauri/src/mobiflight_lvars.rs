//! Puente LVar **opcional** vía el módulo WASM de MobiFlight.
//!
//! ## Por qué existe
//!
//! MSFS NO expone como *simvar* nativo ni la temperatura de frenos ni
//! ciertos datos de GSX (p. ej. el gate al que el avión está asignado
//! cuando el aeropuerto no tiene perfil de GSX). Esos datos viven como
//! **LVars** (`L:NOMBRE`), y un cliente SimConnect externo —como esta
//! app— **no puede leer LVars directamente**: los LVars solo son
//! accesibles desde dentro del sim (módulos WASM/gauges).
//!
//! El estándar de facto para puentear LVars hacia un cliente SimConnect
//! es el **módulo WASM de MobiFlight**
//! (<https://github.com/MobiFlight/MobiFlight-WASM-Module>), que el
//! usuario instala en su carpeta `Community`. Ese módulo crea tres
//! *Client Data Areas* sobre SimConnect:
//!
//!   · `MobiFlight.Command`  — el cliente ESCRIBE comandos de texto.
//!   · `MobiFlight.Response` — el módulo escribe respuestas (no lo
//!                             usamos en este flujo mínimo).
//!   · `MobiFlight.LVars`    — el módulo streamea los valores (floats)
//!                             de las variables registradas, en el
//!                             orden en que se registraron.
//!
//! ## Qué leemos
//!
//!   1. **Brake temps** del FlightByWire A32NX (4 LVars en °C).
//!   2. **Gate de GSX** — el "subtítulo" del menú de GSX. Desde GSX Pro
//!      ~4.0.x el gate ("Gate B 14") ya NO va en el título del menú (que
//!      se escribe a disco) sino en un slot de string codificado en
//!      LVars: `L:FSDT_GSX_MENU_SUBTITLE_LEN` (nº de chars base64) +
//!      `L:FSDT_GSX_MENU_SUBTITLE_B0..` (4 chars base64 por chunk, 6 bits
//!      por char, char bajo primero). Verificado leyendo el JS del panel
//!      de GSX (`readStringSlot`, `CHARS_PER_CHUNK = 4`, alfabeto base64
//!      estándar, `MENU_SUBTITLE_MAX_CHUNKS = 40`). Decodificamos el
//!      string aquí; el watcher lo usa como gate cuando el menú principal
//!      ("Activate Services at") está abierto.
//!
//! ## Protocolo que usamos (mínimo)
//!
//!   1. `MapClientDataNameToID` para `MobiFlight.Command` y
//!      `MobiFlight.LVars`.
//!   2. Definir el área de comandos (1 string de 1024 bytes).
//!   3. `SetClientData(Command) = "MF.SimVars.Clear"` para resetear el
//!      registro, luego `"MF.SimVars.Add.(L:...)"` por cada LVar.
//!   4. Definir el área de LVars: un `FLOAT32` por variable en offsets
//!      0, 4, 8, … y `RequestClientData` con periodo SECOND.
//!   5. Leer los floats en el dispatch `RECV_ID_CLIENT_DATA`.
//!
//! ## Degradación elegante
//!
//! Si el módulo WASM NO está instalado, los `Map/Set/Request` simplemente
//! no producen datos y todo queda `None`. NADA se rompe: el resto del
//! watcher (detección de vuelos, OOOI, FPM, etc.) es independiente.

#![cfg(target_os = "windows")]
#![allow(clippy::missing_safety_doc)]

use std::ffi::c_void;

use base64::Engine as _;

use crate::simconnect_ffi as sc;
use crate::simconnect_ffi::SimConnectLib;

// IDs locales del cliente. Elegidos en un rango alto y distintivo
// ("MF" = 0x4D46) para no colisionar con los DEFINE_ID/REQUEST_ID que
// el watcher usa para simvars y facility data.
const CLIENT_DATA_ID_LVAR: sc::DWORD = 0x4D46_0001;
const CLIENT_DATA_ID_CMD: sc::DWORD = 0x4D46_0002;
const CLIENT_DATA_ID_RESPONSE: sc::DWORD = 0x4D46_0003;
const DEFINE_ID_CMD: sc::DWORD = 0x4D46_0010;
const DEFINE_ID_LVAR: sc::DWORD = 0x4D46_0011;
/// Request id del stream de LVars — el dispatch lo compara para saber
/// que un `RECV_ID_CLIENT_DATA` trae nuestros valores.
pub const REQUEST_ID_LVAR: sc::DWORD = 0x4D46_0020;

const MOBIFLIGHT_COMMAND_AREA: &str = "MobiFlight.Command";
const MOBIFLIGHT_RESPONSE_AREA: &str = "MobiFlight.Response";
const MOBIFLIGHT_LVAR_AREA: &str = "MobiFlight.LVars";
/// Tamaño del área de comandos de MobiFlight (`DATA_STRING_SIZE` en los
/// clientes de referencia). Los comandos son cortos; 256 sobra.
const MOBIFLIGHT_MESSAGE_SIZE: sc::DWORD = 256;
/// Tamaño del área de LVars de MobiFlight (4096 bytes = 1024 floats).
const MOBIFLIGHT_LVAR_AREA_SIZE: sc::DWORD = 4096;

/// LVars de temperatura de frenos que registramos, **en orden**. El
/// offset de cada uno en el área de LVars = índice × 4 bytes.
///
/// FBW A32NX expone `A32NX_REPORTED_BRAKE_TEMPERATURE_{1..4}` en °C
/// (lo que muestra el ECAM). Para añadir otro avión: agregar su(s)
/// expresión(es) RPN aquí; los offsets se recalculan solos.
pub const BRAKE_TEMP_LVARS: &[&str] = &[
    "(L:A32NX_REPORTED_BRAKE_TEMPERATURE_1)",
    "(L:A32NX_REPORTED_BRAKE_TEMPERATURE_2)",
    "(L:A32NX_REPORTED_BRAKE_TEMPERATURE_3)",
    "(L:A32NX_REPORTED_BRAKE_TEMPERATURE_4)",
];

/// Nº de chunks `_Bk` del slot de subtítulo de GSX que registramos. GSX
/// soporta hasta 40 (`MENU_SUBTITLE_MAX_CHUNKS`); con 16 chunks × 4 chars
/// = 64 chars base64 = 48 bytes, más que de sobra para cualquier nombre
/// de gate/parking ("N Cargo Apron 12" cabe holgado).
const GSX_SUBTITLE_CHUNKS: usize = 16;
/// Chars base64 por chunk (lo fija GSX en su panel: `CHARS_PER_CHUNK`).
const GSX_SUBTITLE_CHARS_PER_CHUNK: usize = 4;
/// Alfabeto base64 estándar (el que usa `readStringSlot` de GSX).
const B64_STD: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Estado del puente tras configurarlo. El watcher lo guarda para
/// interpretar los dispatches de Client Data.
pub struct LvarBridge {
    pub request_id: sc::DWORD,
    /// Total de floats registrados (brake temps + LEN + chunks).
    pub var_count: usize,
    /// Los primeros `brake_count` floats son brake temps (°C).
    pub brake_count: usize,
    /// Índice del float `L:FSDT_GSX_MENU_SUBTITLE_LEN`.
    pub subtitle_len_index: usize,
    /// Nº de floats chunk `_Bk` que siguen al `_LEN`.
    pub subtitle_chunk_count: usize,
}

/// Configura el puente sobre una conexión SimConnect ya abierta.
/// Devuelve `None` si la DLL no exporta las funciones de Client Data
/// (entonces el puente queda deshabilitado, sin romper nada).
///
/// # Safety
/// `handle` debe ser un `HSIMCONNECT` válido y abierto.
pub unsafe fn setup(lib: &SimConnectLib, handle: sc::HANDLE) -> Option<LvarBridge> {
    let (Some(map), Some(create), Some(add_def), Some(set), Some(req)) = (
        lib.MapClientDataNameToID,
        lib.CreateClientData,
        lib.AddToClientDataDefinition,
        lib.SetClientData,
        lib.RequestClientData,
    ) else {
        tracing::info!(
            target: "lvar_bridge",
            "SimConnect.dll no exporta las funciones de Client Data; puente LVar deshabilitado"
        );
        return None;
    };

    let brake_count = BRAKE_TEMP_LVARS.len();
    let subtitle_len_index = brake_count;
    let var_count = brake_count + 1 + GSX_SUBTITLE_CHUNKS;

    // 1) Mapear + CREAR las 3 áreas de MobiFlight (LVars, Command,
    //    Response). Los clientes de referencia "raw" (vía SimConnect.dll,
    //    como nosotros) crean las áreas explícitamente — sin esto el
    //    stream de LVars no llega.
    let lvar_name = sc::cstr(MOBIFLIGHT_LVAR_AREA);
    let cmd_name = sc::cstr(MOBIFLIGHT_COMMAND_AREA);
    let resp_name = sc::cstr(MOBIFLIGHT_RESPONSE_AREA);
    map(handle, lvar_name.as_ptr(), CLIENT_DATA_ID_LVAR);
    create(handle, CLIENT_DATA_ID_LVAR, MOBIFLIGHT_LVAR_AREA_SIZE, 0);
    map(handle, cmd_name.as_ptr(), CLIENT_DATA_ID_CMD);
    create(handle, CLIENT_DATA_ID_CMD, MOBIFLIGHT_MESSAGE_SIZE, 0);
    map(handle, resp_name.as_ptr(), CLIENT_DATA_ID_RESPONSE);
    create(handle, CLIENT_DATA_ID_RESPONSE, MOBIFLIGHT_MESSAGE_SIZE, 0);

    // 2) Definir el área de comandos: un único string en offset 0.
    add_def(handle, DEFINE_ID_CMD, 0, MOBIFLIGHT_MESSAGE_SIZE, 0.0, 0);

    // 3) Registrar nuestros LVars EN ORDEN. Mandamos un `MF.Ping` primero
    //    porque el módulo a veces ignora el PRIMER comando; luego Clear y
    //    los Add: brake temps, después el slot de subtítulo de GSX (LEN +
    //    chunks). El orden define los offsets en el área de LVars.
    send_command(set, handle, "MF.Ping");
    send_command(set, handle, "MF.SimVars.Clear");
    for expr in BRAKE_TEMP_LVARS {
        send_command(set, handle, &format!("MF.SimVars.Add.{expr}"));
    }
    send_command(
        set,
        handle,
        "MF.SimVars.Add.(L:FSDT_GSX_MENU_SUBTITLE_LEN)",
    );
    for i in 0..GSX_SUBTITLE_CHUNKS {
        send_command(
            set,
            handle,
            &format!("MF.SimVars.Add.(L:FSDT_GSX_MENU_SUBTITLE_B{i})"),
        );
    }

    // 4) Definir el área de LVars: un FLOAT32 por variable, offsets 0,4,8…
    for i in 0..var_count {
        add_def(
            handle,
            DEFINE_ID_LVAR,
            (i as sc::DWORD) * 4,
            sc::SIMCONNECT_CLIENTDATATYPE_FLOAT32,
            0.0,
            i as sc::DWORD,
        );
    }

    // 5) Suscribirse al stream de valores con PERIOD_ON_SET: el módulo
    //    ESCRIBE el área y SimConnect nos lo entrega. `SECOND` no dispara
    //    para escrituras de otro cliente — ese era el bug.
    req(
        handle,
        CLIENT_DATA_ID_LVAR,
        REQUEST_ID_LVAR,
        DEFINE_ID_LVAR,
        sc::SIMCONNECT_CLIENT_DATA_PERIOD_ON_SET,
        sc::SIMCONNECT_CLIENT_DATA_REQUEST_FLAG_DEFAULT,
        0,
        0,
        0,
    );

    tracing::info!(
        target: "lvar_bridge",
        "puente LVar de MobiFlight configurado ({} vars: {} brake-temp + gate de GSX). Si no llega nada, falta el módulo WASM de MobiFlight en Community.",
        var_count, brake_count
    );
    Some(LvarBridge {
        request_id: REQUEST_ID_LVAR,
        var_count,
        brake_count,
        subtitle_len_index,
        subtitle_chunk_count: GSX_SUBTITLE_CHUNKS,
    })
}

/// Escribe un comando de texto en el área `MobiFlight.Command`.
unsafe fn send_command(set: sc::FnSetClientData, handle: sc::HANDLE, cmd: &str) {
    // Buffer de tamaño fijo (el área entera), null-terminado.
    let mut buf = vec![0u8; MOBIFLIGHT_MESSAGE_SIZE as usize];
    let bytes = cmd.as_bytes();
    let n = bytes.len().min(buf.len() - 1);
    buf[..n].copy_from_slice(&bytes[..n]);
    set(
        handle,
        CLIENT_DATA_ID_CMD,
        DEFINE_ID_CMD,
        sc::SIMCONNECT_CLIENT_DATA_SET_FLAG_DEFAULT,
        0,
        MOBIFLIGHT_MESSAGE_SIZE,
        buf.as_ptr() as *const c_void,
    );
}

/// Lee el float en el índice `i` del payload de un `RECV_ID_CLIENT_DATA`.
///
/// # Safety
/// `p_data` debe apuntar a un `SIMCONNECT_RECV_CLIENT_DATA` válido cuyo
/// define corresponda a al menos `i+1` floats contiguos.
unsafe fn read_float(p_data: *const sc::SIMCONNECT_RECV_CLIENT_DATA, i: usize) -> f64 {
    // El payload empieza en `dwData` (marcador [DWORD;1] al final del
    // struct) — mismo cálculo que el dump de GSX en el watcher.
    let base = (p_data as *const u8)
        .add(std::mem::size_of::<sc::SIMCONNECT_RECV_CLIENT_DATA>() - 4);
    (base.add(i * 4) as *const f32).read_unaligned() as f64
}

/// Lee los primeros `count` floats y devuelve la temperatura de frenos
/// MÁXIMA válida (°C), o `None` si todos son 0 (LVar inexistente / frenos
/// fríos) o fuera de rango.
///
/// # Safety
/// `p_data` debe apuntar a un `SIMCONNECT_RECV_CLIENT_DATA` válido con al
/// menos `count` floats.
pub unsafe fn parse_max_brake_temp(
    p_data: *const sc::SIMCONNECT_RECV_CLIENT_DATA,
    count: usize,
) -> Option<f64> {
    let mut max: Option<f64> = None;
    for i in 0..count {
        let f = read_float(p_data, i);
        // Ignoramos 0 (LVar ausente) y valores absurdos. Un freno real
        // ronda 20-40°C en frío y puede pasar de 300°C tras frenar.
        if f > 1.0 && f < 3000.0 {
            max = Some(max.map_or(f, |m| m.max(f)));
        }
    }
    max
}

/// Decodifica el slot de subtítulo del menú de GSX (LEN + chunks base64)
/// y devuelve el string (p. ej. `"Gate B 14"`), o `None` si el slot está
/// vacío (menú sin subtítulo / sin gate) o no decodifica.
///
/// Replica `readStringSlot` del panel de GSX: cada chunk lleva
/// `CHARS_PER_CHUNK` chars base64 empaquetados 6 bits por char (char bajo
/// primero) como `Σ idx_j · 64^j`. Los chunks son enteros ≤ 64^4-1 =
/// 16_777_215, exactamente representables en f32.
///
/// # Safety
/// `p_data` debe apuntar a un `SIMCONNECT_RECV_CLIENT_DATA` válido con al
/// menos `len_index + 1 + chunk_count` floats.
pub unsafe fn parse_gate_subtitle(
    p_data: *const sc::SIMCONNECT_RECV_CLIENT_DATA,
    len_index: usize,
    chunk_count: usize,
) -> Option<String> {
    let len_f = read_float(p_data, len_index);
    if !len_f.is_finite() || len_f < 1.0 {
        return None;
    }
    let max_chars = chunk_count * GSX_SUBTITLE_CHARS_PER_CHUNK;
    let total = (len_f.round() as usize).min(max_chars);
    if total == 0 {
        return None;
    }
    let mut b64 = String::with_capacity(total + 4);
    let mut remaining = total;
    for i in 0..chunk_count {
        if remaining == 0 {
            break;
        }
        let mut value = read_float(p_data, len_index + 1 + i).round() as u64;
        let count = remaining.min(GSX_SUBTITLE_CHARS_PER_CHUNK);
        for _ in 0..count {
            let idx = (value % 64) as usize;
            b64.push(B64_STD[idx] as char);
            value /= 64;
        }
        remaining -= count;
    }
    // GSX quita el padding '=' al escribir; lo re-añadimos para decodificar.
    while b64.len() % 4 != 0 {
        b64.push('=');
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(&b64).ok()?;
    let s = String::from_utf8_lossy(&bytes).trim().to_string();
    // Rechazar lecturas parciales/torn: el módulo puede entregarnos un
    // LEN>0 con los chunks aún en 0 (mitad de escritura), lo que produce
    // bytes nulos / de control. Eso no es un nombre de gate válido.
    if s.is_empty() || s.chars().any(|c| c.is_control()) {
        None
    } else {
        Some(s)
    }
}
