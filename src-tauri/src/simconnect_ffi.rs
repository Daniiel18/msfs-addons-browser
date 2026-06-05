//! FFI mínimo a `SimConnect.dll` (MSFS 2020/2024).
//!
//! Escribimos las signatures a mano para evitar la dep de
//! `bindgen`/libclang que arrastran los crates `simconnect` y
//! `simconnect-sdk`. Nos basta con:
//!
//!   · `SimConnect_Open` / `SimConnect_Close` — handshake.
//!   · `SimConnect_AddToDataDefinition` — registrar simvars.
//!   · `SimConnect_RequestDataOnSimObject` — suscribirse al user
//!     aircraft con periodo `SECOND`.
//!   · `SimConnect_GetNextDispatch` — pull-based polling.
//!
//! El layout del struct `SIMCONNECT_RECV_SIMOBJECT_DATA` está
//! tomado directo del header `SimConnect.h` de la SDK pública de
//! Microsoft. No es complejo — DWORDs cabezales (DWORD = u32) +
//! data values después.
//!
//! ## Plataforma
//!
//! Sólo Windows (`#[cfg(target_os = "windows")]`). En el resto
//! exponemos stubs que devuelven errors — el watcher cae a la
//! detección por proceso + SimBrief.

#![allow(non_snake_case, non_camel_case_types, dead_code)]

use std::ffi::{c_char, c_void, CString};

pub type HRESULT = i32;
pub type DWORD = u32;
pub type HWND = *mut c_void;
pub type HANDLE = *mut c_void;

// ----- Constantes SimConnect -------------------------------------------------

pub const SIMCONNECT_DATATYPE_FLOAT64: DWORD = 4;
// (v3.4.2 hotfix) ¡OJO! Los IDs del enum SimConnectDataType en
// SimConnect.h del SDK oficial son:
//   FLOAT64 = 4, STRING8 = 5, STRING32 = 6, STRING64 = 7,
//   STRING128 = 8, STRING256 = 9, STRING260 = 10, STRINGV = 11.
// La primera versión de este file tenía STRING256 = 11 (= STRINGV
// variable-length, layout incompatible) — eso causaba que el sim
// rechazara nuestro AddToDataDefinition con DEFINITION_ERROR
// (code=28) en cuanto procesaba el primer string simvar tras
// los FLOAT64. El bug se manifestaba como "el watcher arranca
// pero los gates nunca aparecen" porque las EXCEPTIONS dejaban
// el define corrupto y los siguientes RECV_FACILITY_DATA quedaban
// silenciados. Valor correcto: 9.
pub const SIMCONNECT_DATATYPE_STRING256: DWORD = 9;

pub const SIMCONNECT_PERIOD_NEVER: DWORD = 0;
pub const SIMCONNECT_PERIOD_ONCE: DWORD = 1;
pub const SIMCONNECT_PERIOD_VISUAL_FRAME: DWORD = 2;
pub const SIMCONNECT_PERIOD_SIM_FRAME: DWORD = 3;
pub const SIMCONNECT_PERIOD_SECOND: DWORD = 4;

pub const SIMCONNECT_OBJECT_ID_USER: DWORD = 0;

pub const SIMCONNECT_RECV_ID_NULL: DWORD = 0;
pub const SIMCONNECT_RECV_ID_EXCEPTION: DWORD = 1;
pub const SIMCONNECT_RECV_ID_OPEN: DWORD = 2;
pub const SIMCONNECT_RECV_ID_QUIT: DWORD = 3;
pub const SIMCONNECT_RECV_ID_EVENT: DWORD = 4;
pub const SIMCONNECT_RECV_ID_SIMOBJECT_DATA: DWORD = 8;
pub const SIMCONNECT_RECV_ID_SIMOBJECT_DATA_BYTYPE: DWORD = 9;
/// (v3.4.4 hotfix) Los IDs de RECV están en el enum
/// `SIMCONNECT_RECV_ID` del SDK oficial. La versión original de este
/// file tenía valores incorrectos para CLIENT_DATA (=20), FACILITY_DATA
/// (=33) y FACILITY_DATA_END (=34) — esos son los slots de
/// JETWAY_DATA, CONTROLLERS_LIST y ACTION_CALLBACK respectivamente.
/// SimConnect emite los eventos con los IDs reales (29/30), nuestro
/// match no los captura y caen al branch `_ => ignorar`. Resultado:
/// las requests de Facility Data se mandan correctamente pero la
/// respuesta se descarta silenciosamente.
///
/// Valores correctos del enum SimConnectRecvId (MSFS 2020/2024 SDK):
///   NULL=0, EXCEPTION=1, OPEN=2, QUIT=3, EVENT=4,
///   EVENT_OBJECT_ADDREMOVE=5, EVENT_FILENAME=6, EVENT_FRAME=7,
///   SIMOBJECT_DATA=8, SIMOBJECT_DATA_BYTYPE=9,
///   WEATHER_OBSERVATION=10, CLOUD_STATE=11, ASSIGNED_OBJECT_ID=12,
///   RESERVED_KEY=13, CUSTOM_ACTION=14, SYSTEM_STATE=15,
///   CLIENT_DATA=16, EVENT_WEATHER_MODE=17, AIRPORT_LIST=18,
///   VOR_LIST=19, NDB_LIST=20, WAYPOINT_LIST=21,
///   EVENT_MULTIPLAYER_*=22..24, EVENT_RACE_*=25..26,
///   OBSERVER_DATA=27, EVENT_EX1=28, FACILITY_DATA=29,
///   FACILITY_DATA_END=30, FACILITY_MINIMAL_LIST=31,
///   JETWAY_DATA=32, CONTROLLERS_LIST=33, ACTION_CALLBACK=34.
pub const SIMCONNECT_RECV_ID_CLIENT_DATA: DWORD = 16;
/// Emitido por cada nodo de la jerarquía AIRPORT (apertura, cada
/// child como TAXI_PARKING, cierre).
pub const SIMCONNECT_RECV_ID_FACILITY_DATA: DWORD = 29;
/// Emitido cuando la jerarquía completa de una request termina de
/// transmitirse. El watcher usa esta señal para procesar la lista
/// de parkings recolectados y elegir el más cercano al player.
pub const SIMCONNECT_RECV_ID_FACILITY_DATA_END: DWORD = 30;

pub const SIMCONNECT_DATA_REQUEST_FLAG_DEFAULT: DWORD = 0;
pub const SIMCONNECT_DATA_REQUEST_FLAG_CHANGED: DWORD = 1;

// ----- Client Data constants (v4.0.0 — puente LVar de MobiFlight) -----
//
// `SimConnect_AddToClientDataDefinition` acepta como `dwSizeOrType` o
// bien un tamaño en bytes (positivo) o bien uno de los enum
// `SIMCONNECT_CLIENTDATATYPE_*` (negativos). FLOAT32 = -5; lo pasamos
// como DWORD en complemento a dos (0xFFFFFFFB).
pub const SIMCONNECT_CLIENTDATATYPE_FLOAT32: DWORD = (-5i32) as DWORD;
/// Periodo de stream del Client Data (enum SIMCONNECT_CLIENT_DATA_PERIOD).
/// SECOND = 4. Brake temps cambian lento, 1 Hz sobra.
pub const SIMCONNECT_CLIENT_DATA_PERIOD_SECOND: DWORD = 4;
/// Flag por defecto en RequestClientData.
pub const SIMCONNECT_CLIENT_DATA_REQUEST_FLAG_DEFAULT: DWORD = 0;
/// Flag por defecto en SetClientData.
pub const SIMCONNECT_CLIENT_DATA_SET_FLAG_DEFAULT: DWORD = 0;

// SimConnect_Open `ConfigIndex` parameter — un valor 0 indica
// "config local default" que es lo que necesitamos.
pub const SIMCONNECT_OPEN_CONFIGINDEX_LOCAL: DWORD = 0;

// ----- Estructuras -----------------------------------------------------------

#[repr(C, packed(4))]
pub struct SIMCONNECT_RECV {
    pub dwSize: DWORD,
    pub dwVersion: DWORD,
    pub dwID: DWORD,
}

#[repr(C, packed(4))]
pub struct SIMCONNECT_RECV_SIMOBJECT_DATA {
    pub dwSize: DWORD,
    pub dwVersion: DWORD,
    pub dwID: DWORD,
    pub dwRequestID: DWORD,
    pub dwObjectID: DWORD,
    pub dwDefineID: DWORD,
    pub dwFlags: DWORD,
    pub dwentrynumber: DWORD,
    pub dwoutof: DWORD,
    pub dwDefineCount: DWORD,
    /// Marker — los datos reales vienen inmediatamente después.
    /// Calculamos su offset con `size_of::<SIMCONNECT_RECV_SIMOBJECT_DATA>()`.
    pub dwData: [DWORD; 1],
}

#[repr(C, packed(4))]
pub struct SIMCONNECT_RECV_EXCEPTION {
    pub dwSize: DWORD,
    pub dwVersion: DWORD,
    pub dwID: DWORD,
    pub dwException: DWORD,
    pub dwSendID: DWORD,
    pub dwIndex: DWORD,
}

/// `SIMCONNECT_RECV_EVENT` — un evento de sistema (v0.1.25, usado
/// para detectar pausa). El campo `dwData` es 0/1 (o el valor numérico
/// específico del evento). Para "Pause", `dwData == 1` significa
/// "simulator paused", `dwData == 0` significa "resumed".
#[repr(C, packed(4))]
pub struct SIMCONNECT_RECV_EVENT {
    pub dwSize: DWORD,
    pub dwVersion: DWORD,
    pub dwID: DWORD,
    pub uGroupID: DWORD,
    pub uEventID: DWORD,
    pub dwData: DWORD,
}

/// `SIMCONNECT_RECV_FACILITY_DATA` (v0.1.26) — un nodo de la
/// jerarquía AIRPORT solicitada vía `RequestFacilityData_EX1`.
///
/// `Type` indica el tipo de nodo (apertura del airport, taxi parking,
/// runway, cierre, etc). Los bytes después de `Data[0]` son los
/// campos que pediste vía `AddToFacilityDefinition` en orden.
#[repr(C, packed(4))]
pub struct SIMCONNECT_RECV_FACILITY_DATA {
    pub dwSize: DWORD,
    pub dwVersion: DWORD,
    pub dwID: DWORD,
    pub UserRequestId: DWORD,
    pub UniqueRequestId: DWORD,
    pub ParentUniqueRequestId: DWORD,
    pub Type: DWORD,
    pub IsListItem: DWORD,
    pub ItemIndex: DWORD,
    pub ListSize: DWORD,
    /// Marker — los datos reales comienzan en este offset.
    pub Data: [DWORD; 1],
}

/// `SIMCONNECT_RECV_FACILITY_DATA_END` — disparado cuando la
/// jerarquía completa de la request terminó de enviarse.
#[repr(C, packed(4))]
pub struct SIMCONNECT_RECV_FACILITY_DATA_END {
    pub dwSize: DWORD,
    pub dwVersion: DWORD,
    pub dwID: DWORD,
    pub RequestId: DWORD,
}

/// `SIMCONNECT_RECV_CLIENT_DATA` (v0.1.26) — payload de un Client
/// Data Area. Usado para leer los datos que el módulo WASM de GSX
/// expone (boarding state, pax count, etc.).
#[repr(C, packed(4))]
pub struct SIMCONNECT_RECV_CLIENT_DATA {
    pub dwSize: DWORD,
    pub dwVersion: DWORD,
    pub dwID: DWORD,
    pub dwRequestID: DWORD,
    pub dwObjectID: DWORD,
    pub dwDefineID: DWORD,
    pub dwFlags: DWORD,
    pub dwentrynumber: DWORD,
    pub dwoutof: DWORD,
    pub dwDefineCount: DWORD,
    pub dwData: [DWORD; 1],
}

// Facility data types — valores del enum `SIMCONNECT_FACILITY_DATA_TYPE`
// según SimConnect.h. **OJO** (bug fix v1.1.2): en versiones previas
// teníamos `TAXI_PARKING = 4`, lo cual es `HELIPAD` — por eso los
// gates reales nunca se detectaban y se quedaban en el fallback feo
// "Stand · X° Ym de ICAO".
pub const SIMCONNECT_FACILITY_DATA_AIRPORT: DWORD = 0;
pub const SIMCONNECT_FACILITY_DATA_RUNWAY: DWORD = 1;
pub const SIMCONNECT_FACILITY_DATA_START: DWORD = 2;
pub const SIMCONNECT_FACILITY_DATA_FREQUENCY: DWORD = 3;
pub const SIMCONNECT_FACILITY_DATA_HELIPAD: DWORD = 4;
pub const SIMCONNECT_FACILITY_DATA_TAXI_POINT: DWORD = 14;
pub const SIMCONNECT_FACILITY_DATA_TAXI_PARKING: DWORD = 15;
pub const SIMCONNECT_FACILITY_DATA_TAXI_PATH: DWORD = 16;

// ----- Function pointer types -----------------------------------------------

pub type FnOpen = unsafe extern "system" fn(
    phSimConnect: *mut HANDLE,
    szName: *const c_char,
    hWnd: HWND,
    UserEventWin32: DWORD,
    hEventHandle: HANDLE,
    ConfigIndex: DWORD,
) -> HRESULT;

pub type FnClose = unsafe extern "system" fn(hSimConnect: HANDLE) -> HRESULT;

pub type FnAddToDataDefinition = unsafe extern "system" fn(
    hSimConnect: HANDLE,
    DefineID: DWORD,
    DatumName: *const c_char,
    UnitsName: *const c_char,
    DatumType: DWORD,
    fEpsilon: f32,
    DatumID: DWORD,
) -> HRESULT;

pub type FnRequestDataOnSimObject = unsafe extern "system" fn(
    hSimConnect: HANDLE,
    RequestID: DWORD,
    DefineID: DWORD,
    ObjectID: DWORD,
    Period: DWORD,
    Flags: DWORD,
    origin: DWORD,
    interval: DWORD,
    limit: DWORD,
) -> HRESULT;

pub type FnGetNextDispatch = unsafe extern "system" fn(
    hSimConnect: HANDLE,
    ppData: *mut *mut SIMCONNECT_RECV,
    pcbData: *mut DWORD,
) -> HRESULT;

/// `SimConnect_SubscribeToSystemEvent` — registra un EventID para
/// que llegue cuando el sim emita el evento sistema indicado
/// (`SystemEventName`). Nombres usados:
///   · "Pause"   — fires con dwData = 1 al pausar, 0 al despausar.
///   · "Paused"  — fires SÓLO cuando se pausa (one-shot).
///   · "Unpaused"— fires SÓLO cuando se despausa (one-shot).
///   · "Sim"     — fires con dwData = 1 cuando arranca un vuelo
///                 (estado "running"), 0 al "stopping".
pub type FnSubscribeToSystemEvent = unsafe extern "system" fn(
    hSimConnect: HANDLE,
    EventID: DWORD,
    SystemEventName: *const c_char,
) -> HRESULT;

// ───── Facility Data API (v0.1.26 — gates reales) ─────

/// `SimConnect_AddToFacilityDefinition` — define qué campos quieres
/// recibir de la jerarquía AIRPORT. Cada llamada añade un campo
/// (los nombres incluyen marcadores OPEN/CLOSE — ver simconnect_watcher).
pub type FnAddToFacilityDefinition = unsafe extern "system" fn(
    hSimConnect: HANDLE,
    DefineID: DWORD,
    FieldName: *const c_char,
) -> HRESULT;

/// `SimConnect_RequestFacilityData` — dispara la transmisión de los
/// datos del aeropuerto `ICAO` (`Region` puede ser ""). MSFS responde
/// con varios `SIMCONNECT_RECV_FACILITY_DATA` + un único
/// `SIMCONNECT_RECV_FACILITY_DATA_END`.
pub type FnRequestFacilityData = unsafe extern "system" fn(
    hSimConnect: HANDLE,
    DefineID: DWORD,
    RequestID: DWORD,
    ICAO: *const c_char,
    Region: *const c_char,
) -> HRESULT;

// ───── Client Data API (v0.1.26 — GSX WASM bridge) ─────

/// `SimConnect_MapClientDataNameToID` — asocia el nombre del Client
/// Data Area (string) con un ID local que usamos en siguientes calls.
/// El WASM module de GSX define su propia area con un nombre fijo,
/// nosotros mapeamos a un ID local.
pub type FnMapClientDataNameToID = unsafe extern "system" fn(
    hSimConnect: HANDLE,
    szClientDataName: *const c_char,
    ClientDataID: DWORD,
) -> HRESULT;

/// `SimConnect_AddToClientDataDefinition` — define el layout
/// (offsets + sizes) de un struct dentro de un Client Data Area.
pub type FnAddToClientDataDefinition = unsafe extern "system" fn(
    hSimConnect: HANDLE,
    DefineID: DWORD,
    dwOffset: DWORD,
    dwSizeOrType: DWORD,
    fEpsilon: f32,
    DatumID: DWORD,
) -> HRESULT;

/// `SimConnect_RequestClientData` — subscribe al stream de updates
/// del Client Data Area. `Period` igual que SimObject Data (SECOND,
/// SIM_FRAME, etc).
pub type FnRequestClientData = unsafe extern "system" fn(
    hSimConnect: HANDLE,
    ClientDataID: DWORD,
    RequestID: DWORD,
    DefineID: DWORD,
    Period: DWORD,
    Flags: DWORD,
    origin: DWORD,
    interval: DWORD,
    limit: DWORD,
) -> HRESULT;

/// `SimConnect_SetClientData` — escribe bytes en un Client Data Area.
/// (v4.0.0) Lo usamos para enviar comandos de texto al área
/// `MobiFlight.Command` (p.ej. `MF.SimVars.Add.(L:...)`), el mecanismo
/// con el que el módulo WASM de MobiFlight expone LVars sobre
/// SimConnect. `pDataSet` apunta a un buffer de `cbUnitSize` bytes.
pub type FnSetClientData = unsafe extern "system" fn(
    hSimConnect: HANDLE,
    ClientDataID: DWORD,
    DefineID: DWORD,
    Flags: DWORD,
    dwReserved: DWORD,
    cbUnitSize: DWORD,
    pDataSet: *const c_void,
) -> HRESULT;

// ----- Wrapper de carga ------------------------------------------------------

#[cfg(target_os = "windows")]
pub struct SimConnectLib {
    _lib: libloading::Library,
    pub Open: FnOpen,
    pub Close: FnClose,
    pub AddToDataDefinition: FnAddToDataDefinition,
    pub RequestDataOnSimObject: FnRequestDataOnSimObject,
    pub GetNextDispatch: FnGetNextDispatch,
    pub SubscribeToSystemEvent: FnSubscribeToSystemEvent,
    // v0.1.26 — Facility Data + Client Data. Cada uno es Option
    // porque ciertos SDK / DLLs antiguos pueden no exportar el
    // símbolo, y queremos degradar sin fallar al cargar.
    pub AddToFacilityDefinition: Option<FnAddToFacilityDefinition>,
    pub RequestFacilityData: Option<FnRequestFacilityData>,
    pub MapClientDataNameToID: Option<FnMapClientDataNameToID>,
    pub AddToClientDataDefinition: Option<FnAddToClientDataDefinition>,
    pub RequestClientData: Option<FnRequestClientData>,
    pub SetClientData: Option<FnSetClientData>,
}

#[cfg(target_os = "windows")]
impl SimConnectLib {
    /// Carga `SimConnect.dll` probando una larga lista de paths.
    ///
    /// Estrategia (en orden de preferencia):
    ///
    ///   1. **Junto al ejecutable** — la app **bundle** una copia de
    ///      `SimConnect.dll` como Tauri resource. En producción
    ///      vive en `<install-dir>/SimConnect.dll`; en dev en
    ///      `target/debug/SimConnect.dll`. Es el primer candidato
    ///      para que la app funcione "out of the box" aunque el
    ///      usuario no tenga la SDK ni una install de MSFS detectable.
    ///   2. **PATH** del sistema — captura SDKs instaladas que
    ///      añaden el dir al PATH (raro fuera de devs).
    ///   3. **SDKs típicas** — C:\MSFS SDK\, C:\MSFS 2024 SDK\.
    ///   4. **Instalaciones de MSFS** — MS Store (Program Files), Steam
    ///      (Program Files (x86)\Steam\steamapps), MSFS 2024 ídem.
    ///   5. **`%LOCALAPPDATA%\Packages\Microsoft.FlightSimulator*`** —
    ///      donde MSFS guarda recursos compartidos en MS Store.
    ///
    /// Si todo falla devolvemos error con el último mensaje — el
    /// caller (watcher) lo loguea como info (no error, no es fatal)
    /// y cae al fallback proceso+SimBrief.
    pub unsafe fn load() -> anyhow::Result<Self> {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();

        // 1) Junto al ejecutable (bundled resource).
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join("SimConnect.dll"));
                // Algunos bundles ponen el dll en un subdir resources/.
                candidates.push(dir.join("resources").join("SimConnect.dll"));
            }
        }

        // 2) Por nombre desnudo — Windows lo busca en PATH + exe dir.
        candidates.push(std::path::PathBuf::from("SimConnect.dll"));

        // 3) SDKs.
        for p in &[
            r"C:\MSFS SDK\SimConnect SDK\lib\SimConnect.dll",
            r"C:\MSFS 2024 SDK\SimConnect SDK\lib\SimConnect.dll",
        ] {
            candidates.push(std::path::PathBuf::from(p));
        }

        // 4) Instalaciones de MSFS. Probamos ambos Program Files.
        for env_var in &["ProgramFiles", "ProgramFiles(x86)"] {
            let Some(base) = std::env::var_os(env_var) else { continue };
            for sub in &[
                r"Microsoft Flight Simulator\SimConnect.dll",
                r"Microsoft Flight Simulator\SimConnect SDK\lib\SimConnect.dll",
                r"Microsoft Flight Simulator 2024\SimConnect.dll",
                r"Microsoft Flight Simulator 2024\SimConnect SDK\lib\SimConnect.dll",
                r"Steam\steamapps\common\Microsoft Flight Simulator\SimConnect.dll",
                r"Steam\steamapps\common\Microsoft Flight Simulator 2024\SimConnect.dll",
            ] {
                candidates.push(std::path::Path::new(&base).join(sub));
            }
        }

        // 5) MS Store local cache. La ruta exacta varía según versión
        // del paquete UWP, pero el padre `Microsoft.FlightSimulator_*`
        // es estable.
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let local_path = std::path::PathBuf::from(local);
            for pkg in &[
                "Microsoft.FlightSimulator_8wekyb3d8bbwe",
                "Microsoft.Limitless_8wekyb3d8bbwe", // MSFS 2024 MS Store
            ] {
                candidates.push(
                    local_path
                        .join("Packages")
                        .join(pkg)
                        .join("LocalCache")
                        .join("SimConnect.dll"),
                );
            }
        }

        // Probar cada candidato. Loguamos solo los que existen para
        // no inundar el log con paths que nunca van a existir.
        let mut tried: Vec<String> = Vec::new();
        for path in &candidates {
            let path_str = path.to_string_lossy().into_owned();
            let exists = path.is_file()
                // Por nombre desnudo NO podemos saber si existe sin
                // intentar load — lo intentamos siempre.
                || path.components().count() == 1;
            if !exists {
                continue;
            }
            tracing::debug!(target: "simconnect", "intentando SimConnect.dll: {}", path_str);
            match unsafe { libloading::Library::new(path) } {
                Ok(lib) => {
                    tracing::info!(
                        target: "simconnect",
                        "SimConnect.dll cargada desde: {}",
                        path_str
                    );
                    return Self::from_library(lib);
                }
                Err(e) => {
                    tried.push(format!("{}: {}", path_str, e));
                }
            }
        }

        Err(anyhow::anyhow!(
            "SimConnect.dll no se pudo cargar. Paths probados:\n{}",
            if tried.is_empty() {
                "(ningún candidato existía en disco)".to_string()
            } else {
                tried.join("\n")
            }
        ))
    }

    /// Resuelve los símbolos contra una `Library` ya cargada. Separar
    /// esto del `load()` mantiene la búsqueda multi-path manejable.
    unsafe fn from_library(lib: libloading::Library) -> anyhow::Result<Self> {
        let open: libloading::Symbol<FnOpen> = lib.get(b"SimConnect_Open\0")?;
        let close: libloading::Symbol<FnClose> = lib.get(b"SimConnect_Close\0")?;
        let add_def: libloading::Symbol<FnAddToDataDefinition> =
            lib.get(b"SimConnect_AddToDataDefinition\0")?;
        let req: libloading::Symbol<FnRequestDataOnSimObject> =
            lib.get(b"SimConnect_RequestDataOnSimObject\0")?;
        let get_next: libloading::Symbol<FnGetNextDispatch> =
            lib.get(b"SimConnect_GetNextDispatch\0")?;
        let subscribe: libloading::Symbol<FnSubscribeToSystemEvent> =
            lib.get(b"SimConnect_SubscribeToSystemEvent\0")?;

        let open_fn = *open;
        let close_fn = *close;
        let add_def_fn = *add_def;
        let req_fn = *req;
        let get_next_fn = *get_next;
        let subscribe_fn = *subscribe;

        // Optional v0.1.26 symbols — degradan a None si la DLL no
        // los expone (versiones más antiguas, MSFS 2020 antes de
        // algún SU). El watcher chequea Option antes de llamar.
        let add_fac_def: Option<FnAddToFacilityDefinition> = lib
            .get::<FnAddToFacilityDefinition>(b"SimConnect_AddToFacilityDefinition\0")
            .ok()
            .map(|s| *s);
        let req_fac_data: Option<FnRequestFacilityData> = lib
            .get::<FnRequestFacilityData>(b"SimConnect_RequestFacilityData\0")
            .ok()
            .map(|s| *s);
        let map_client: Option<FnMapClientDataNameToID> = lib
            .get::<FnMapClientDataNameToID>(b"SimConnect_MapClientDataNameToID\0")
            .ok()
            .map(|s| *s);
        let add_client_def: Option<FnAddToClientDataDefinition> = lib
            .get::<FnAddToClientDataDefinition>(b"SimConnect_AddToClientDataDefinition\0")
            .ok()
            .map(|s| *s);
        let req_client: Option<FnRequestClientData> = lib
            .get::<FnRequestClientData>(b"SimConnect_RequestClientData\0")
            .ok()
            .map(|s| *s);
        let set_client: Option<FnSetClientData> = lib
            .get::<FnSetClientData>(b"SimConnect_SetClientData\0")
            .ok()
            .map(|s| *s);

        Ok(Self {
            _lib: lib,
            Open: open_fn,
            Close: close_fn,
            AddToDataDefinition: add_def_fn,
            RequestDataOnSimObject: req_fn,
            GetNextDispatch: get_next_fn,
            SubscribeToSystemEvent: subscribe_fn,
            AddToFacilityDefinition: add_fac_def,
            RequestFacilityData: req_fac_data,
            MapClientDataNameToID: map_client,
            AddToClientDataDefinition: add_client_def,
            RequestClientData: req_client,
            SetClientData: set_client,
        })
    }
}

// ----- Helpers ---------------------------------------------------------------

/// Convierte una `&str` Rust en `CString` para pasar a la API C.
/// Usar siempre `as_ptr()` justo antes de la llamada y mantener el
/// `CString` vivo durante la duración de la llamada (los punteros
/// se invalidan al drop).
pub fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
}

/// True si el HRESULT representa éxito (S_OK = 0). SimConnect usa
/// HRESULTs estándar de Windows.
pub fn succeeded(hr: HRESULT) -> bool {
    hr >= 0
}

// Re-exports de las constantes para que el watcher sólo importe
// este módulo y no `simconnect_ffi::*`.
pub mod consts {
    pub use super::{
        SIMCONNECT_DATATYPE_FLOAT64, SIMCONNECT_DATA_REQUEST_FLAG_DEFAULT,
        SIMCONNECT_FACILITY_DATA_AIRPORT, SIMCONNECT_FACILITY_DATA_RUNWAY,
        SIMCONNECT_FACILITY_DATA_TAXI_PARKING, SIMCONNECT_OBJECT_ID_USER,
        SIMCONNECT_OPEN_CONFIGINDEX_LOCAL, SIMCONNECT_PERIOD_SECOND,
        SIMCONNECT_PERIOD_SIM_FRAME, SIMCONNECT_RECV_ID_CLIENT_DATA,
        SIMCONNECT_RECV_ID_EVENT, SIMCONNECT_RECV_ID_EXCEPTION,
        SIMCONNECT_RECV_ID_FACILITY_DATA, SIMCONNECT_RECV_ID_FACILITY_DATA_END,
        SIMCONNECT_RECV_ID_OPEN, SIMCONNECT_RECV_ID_QUIT,
        SIMCONNECT_RECV_ID_SIMOBJECT_DATA,
    };
}
