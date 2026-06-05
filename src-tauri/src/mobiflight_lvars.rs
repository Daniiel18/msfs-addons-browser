//! Puente LVar **opcional** vía el módulo WASM de MobiFlight.
//!
//! ## Por qué existe
//!
//! MSFS NO expone la temperatura de frenos (ni la mayoría de
//! variables de aviones de estudio) como *simvar* nativo. Esos datos
//! viven como **LVars** (`L:NOMBRE`), y un cliente SimConnect externo
//! —como esta app— **no puede leer LVars directamente**: los LVars
//! solo son accesibles desde dentro del sim (módulos WASM/gauges).
//!
//! El estándar de facto para puentear LVars hacia un cliente
//! SimConnect es el **módulo WASM de MobiFlight**
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
//! no producen datos (a lo sumo alguna EXCEPTION puntual que el watcher
//! loguea) y `max_brake_temp_c` queda `None`. NADA se rompe: el resto
//! del watcher (detección de vuelos, OOOI, FPM, etc.) es independiente.
//!
//! ## Aviones soportados
//!
//! Hoy registramos los LVars de brake temp del **FlightByWire A32NX**
//! (open-source, nombres públicos y estables). PMDG 737 no expone temp
//! de frenos como LVar; otros aviones de estudio se pueden añadir a
//! [`BRAKE_TEMP_LVARS`] cuando se confirmen sus nombres. Los LVars
//! inexistentes en el avión actual evalúan a 0 y los ignoramos.

#![cfg(target_os = "windows")]
#![allow(clippy::missing_safety_doc)]

use std::ffi::c_void;

use crate::simconnect_ffi as sc;
use crate::simconnect_ffi::SimConnectLib;

// IDs locales del cliente. Elegidos en un rango alto y distintivo
// ("MF" = 0x4D46) para no colisionar con los DEFINE_ID/REQUEST_ID que
// el watcher usa para simvars y facility data.
const CLIENT_DATA_ID_CMD: sc::DWORD = 0x4D46_0001;
const CLIENT_DATA_ID_LVAR: sc::DWORD = 0x4D46_0002;
const DEFINE_ID_CMD: sc::DWORD = 0x4D46_0010;
const DEFINE_ID_LVAR: sc::DWORD = 0x4D46_0011;
/// Request id del stream de LVars — el dispatch lo compara para saber
/// que un `RECV_ID_CLIENT_DATA` trae nuestras brake temps.
pub const REQUEST_ID_LVAR: sc::DWORD = 0x4D46_0020;

const MOBIFLIGHT_COMMAND_AREA: &str = "MobiFlight.Command";
const MOBIFLIGHT_LVAR_AREA: &str = "MobiFlight.LVars";
/// Tamaño del área de comandos de MobiFlight (`MOBIFLIGHT_MESSAGE_SIZE`).
const MOBIFLIGHT_MESSAGE_SIZE: sc::DWORD = 1024;

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

/// Estado del puente tras configurarlo. El watcher lo guarda para
/// interpretar los dispatches de Client Data.
pub struct BrakeBridge {
    pub request_id: sc::DWORD,
    pub var_count: usize,
}

/// Configura el puente sobre una conexión SimConnect ya abierta.
/// Devuelve `None` si la DLL no exporta las funciones de Client Data
/// (entonces el puente queda deshabilitado, sin romper nada).
///
/// # Safety
/// `handle` debe ser un `HSIMCONNECT` válido y abierto.
pub unsafe fn setup(lib: &SimConnectLib, handle: sc::HANDLE) -> Option<BrakeBridge> {
    let (Some(map), Some(add_def), Some(set), Some(req)) = (
        lib.MapClientDataNameToID,
        lib.AddToClientDataDefinition,
        lib.SetClientData,
        lib.RequestClientData,
    ) else {
        tracing::info!(
            target: "lvar_bridge",
            "SimConnect.dll no exporta las funciones de Client Data; puente LVar (brake temps) deshabilitado"
        );
        return None;
    };

    // 1) Mapear los nombres de área de MobiFlight a IDs locales.
    let cmd_name = sc::cstr(MOBIFLIGHT_COMMAND_AREA);
    let lvar_name = sc::cstr(MOBIFLIGHT_LVAR_AREA);
    map(handle, cmd_name.as_ptr(), CLIENT_DATA_ID_CMD);
    map(handle, lvar_name.as_ptr(), CLIENT_DATA_ID_LVAR);

    // 2) Definir el área de comandos: un único string de 1024 bytes en
    //    offset 0 (así SetClientData escribe el comando completo).
    add_def(handle, DEFINE_ID_CMD, 0, MOBIFLIGHT_MESSAGE_SIZE, 0.0, 0);

    // 3) Resetear registros previos y registrar nuestros LVars en orden.
    send_command(set, handle, "MF.SimVars.Clear");
    for expr in BRAKE_TEMP_LVARS {
        send_command(set, handle, &format!("MF.SimVars.Add.{expr}"));
    }

    // 4) Definir el área de LVars: un FLOAT32 por variable, offsets 0,4,8…
    for i in 0..BRAKE_TEMP_LVARS.len() {
        add_def(
            handle,
            DEFINE_ID_LVAR,
            (i as sc::DWORD) * 4,
            sc::SIMCONNECT_CLIENTDATATYPE_FLOAT32,
            0.0,
            i as sc::DWORD,
        );
    }

    // 5) Suscribirse al stream de valores (1 Hz).
    req(
        handle,
        CLIENT_DATA_ID_LVAR,
        REQUEST_ID_LVAR,
        DEFINE_ID_LVAR,
        sc::SIMCONNECT_CLIENT_DATA_PERIOD_SECOND,
        sc::SIMCONNECT_CLIENT_DATA_REQUEST_FLAG_DEFAULT,
        0,
        0,
        0,
    );

    tracing::info!(
        target: "lvar_bridge",
        "puente LVar de MobiFlight configurado ({} brake-temp vars). Si no llegan datos, el módulo WASM de MobiFlight no está instalado o el avión no publica esos LVars.",
        BRAKE_TEMP_LVARS.len()
    );
    Some(BrakeBridge {
        request_id: REQUEST_ID_LVAR,
        var_count: BRAKE_TEMP_LVARS.len(),
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

/// Lee los `count` floats del payload de un `RECV_ID_CLIENT_DATA` y
/// devuelve la temperatura de frenos MÁXIMA válida (°C), o `None` si
/// todos son 0 (LVar inexistente / frenos fríos a 0) o fuera de rango.
///
/// # Safety
/// `p_data` debe apuntar a un `SIMCONNECT_RECV_CLIENT_DATA` válido cuyo
/// define corresponda a `count` floats contiguos.
pub unsafe fn parse_max_brake_temp(
    p_data: *const sc::SIMCONNECT_RECV_CLIENT_DATA,
    count: usize,
) -> Option<f64> {
    // El payload empieza en `dwData` (marcador [DWORD;1] al final del
    // struct) — mismo cálculo que el dump de GSX en el watcher.
    let base = (p_data as *const u8)
        .add(std::mem::size_of::<sc::SIMCONNECT_RECV_CLIENT_DATA>() - 4);
    let mut max: Option<f64> = None;
    for i in 0..count {
        let f = (base.add(i * 4) as *const f32).read_unaligned() as f64;
        // Ignoramos 0 (LVar ausente) y valores absurdos. Un freno real
        // ronda 20-40°C en frío y puede pasar de 300°C tras frenar.
        if f > 1.0 && f < 3000.0 {
            max = Some(max.map_or(f, |m| m.max(f)));
        }
    }
    max
}
