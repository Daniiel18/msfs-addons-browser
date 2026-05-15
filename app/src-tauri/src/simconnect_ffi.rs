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
pub const SIMCONNECT_DATATYPE_STRING256: DWORD = 11;

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

pub const SIMCONNECT_DATA_REQUEST_FLAG_DEFAULT: DWORD = 0;
pub const SIMCONNECT_DATA_REQUEST_FLAG_CHANGED: DWORD = 1;

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

// ----- Wrapper de carga ------------------------------------------------------

#[cfg(target_os = "windows")]
pub struct SimConnectLib {
    _lib: libloading::Library,
    pub Open: FnOpen,
    pub Close: FnClose,
    pub AddToDataDefinition: FnAddToDataDefinition,
    pub RequestDataOnSimObject: FnRequestDataOnSimObject,
    pub GetNextDispatch: FnGetNextDispatch,
}

#[cfg(target_os = "windows")]
impl SimConnectLib {
    /// Carga `SimConnect.dll` desde el PATH del sistema. MSFS añade
    /// la SDK al PATH al instalarse, así que `LoadLibrary` resuelve
    /// la dll si MSFS está instalado en cualquiera de sus variantes.
    /// Si no, devolvemos error y el caller cae al fallback.
    pub unsafe fn load() -> anyhow::Result<Self> {
        // Lista de candidatos en orden de probabilidad. Empezamos
        // por el nombre desnudo (PATH) y caemos a paths típicos
        // donde MSFS deja la dll si el PATH no la expone.
        let candidates: &[&str] = &[
            "SimConnect.dll",
            r"C:\MSFS SDK\SimConnect SDK\lib\SimConnect.dll",
            r"C:\MSFS 2024 SDK\SimConnect SDK\lib\SimConnect.dll",
            r"C:\Program Files\Microsoft Flight Simulator\SimConnect SDK\lib\SimConnect.dll",
        ];

        let lib = candidates
            .iter()
            .find_map(|p| {
                tracing::debug!(target: "simconnect", "intentando cargar {}", p);
                unsafe { libloading::Library::new(p).ok() }
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "SimConnect.dll no se encontró en PATH ni en rutas típicas de MSFS SDK"
                )
            })?;

        let open: libloading::Symbol<FnOpen> = lib.get(b"SimConnect_Open\0")?;
        let close: libloading::Symbol<FnClose> = lib.get(b"SimConnect_Close\0")?;
        let add_def: libloading::Symbol<FnAddToDataDefinition> =
            lib.get(b"SimConnect_AddToDataDefinition\0")?;
        let req: libloading::Symbol<FnRequestDataOnSimObject> =
            lib.get(b"SimConnect_RequestDataOnSimObject\0")?;
        let get_next: libloading::Symbol<FnGetNextDispatch> =
            lib.get(b"SimConnect_GetNextDispatch\0")?;

        // Capturamos los punteros y luego dropeamos los `Symbol`s
        // (sus lifetimes están atadas a `lib`). Storeamos `lib` para
        // que la dll permanezca cargada todo el tiempo de vida del
        // wrapper.
        let open_fn = *open;
        let close_fn = *close;
        let add_def_fn = *add_def;
        let req_fn = *req;
        let get_next_fn = *get_next;

        Ok(Self {
            _lib: lib,
            Open: open_fn,
            Close: close_fn,
            AddToDataDefinition: add_def_fn,
            RequestDataOnSimObject: req_fn,
            GetNextDispatch: get_next_fn,
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
        SIMCONNECT_OBJECT_ID_USER, SIMCONNECT_OPEN_CONFIGINDEX_LOCAL,
        SIMCONNECT_PERIOD_SECOND, SIMCONNECT_RECV_ID_EXCEPTION,
        SIMCONNECT_RECV_ID_OPEN, SIMCONNECT_RECV_ID_QUIT,
        SIMCONNECT_RECV_ID_SIMOBJECT_DATA,
    };
}
