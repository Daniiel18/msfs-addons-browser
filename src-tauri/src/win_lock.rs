//! (19/10) Bloqueo de inicio de SimFleet con la cuenta de Windows.
//!
//! Antes de mostrar la app se pide verificar al usuario de Windows:
//!   · Windows Hello (`UserConsentVerifier`): diálogo SEGURO del sistema
//!     (PIN, huella, rostro o contraseña). SimFleet nunca ve la clave.
//!   · Respaldo si Hello no está configurado: contraseña de la cuenta de
//!     Windows validada con `LogonUserW`. La clave sólo vive en memoria el
//!     tiempo de la llamada; no se guarda ni se escribe en logs.

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockInfo {
    /// Windows Hello disponible (PIN/biometría configurados).
    pub hello_available: bool,
    /// Usuario de Windows (sólo para mostrarlo en la pantalla de bloqueo).
    pub username: String,
}

/// Resultado de la verificación con Windows Hello.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HelloOutcome {
    Verified,
    Canceled,
    /// Hello no disponible / no configurado / deshabilitado → usar contraseña.
    Unavailable,
    /// Demasiados intentos fallidos.
    RetriesExhausted,
    Failed,
}

#[tauri::command]
pub async fn lock_info() -> Result<LockInfo, String> {
    let username = std::env::var("USERNAME").unwrap_or_default();
    let hello_available = tokio::task::spawn_blocking(hello_available)
        .await
        .unwrap_or(false);
    Ok(LockInfo {
        hello_available,
        username,
    })
}

#[tauri::command]
pub async fn lock_verify_hello(
    window: tauri::WebviewWindow,
    message: String,
) -> Result<HelloOutcome, String> {
    // El diálogo se ancla a nuestra ventana (si no, en apps de escritorio
    // puede abrirse detrás). El HWND no es Send: pasamos el valor crudo.
    let raw_hwnd = window_handle(&window);
    tokio::task::spawn_blocking(move || verify_hello(raw_hwnd, &message))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn lock_verify_password(password: String) -> Result<bool, String> {
    let ok = tokio::task::spawn_blocking(move || verify_password(&password))
        .await
        .map_err(|e| e.to_string())?;
    if !ok {
        // Freno a intentos en bucle.
        tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    }
    tracing::info!(target: "lock", "verificación por contraseña: {}", if ok { "ok" } else { "fallida" });
    Ok(ok)
}

#[cfg(windows)]
fn window_handle(window: &tauri::WebviewWindow) -> isize {
    window.hwnd().map(|h| h.0 as isize).unwrap_or(0)
}

#[cfg(not(windows))]
fn window_handle(_window: &tauri::WebviewWindow) -> isize {
    0
}

#[cfg(windows)]
fn init_com() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    // S_FALSE / RPC_E_CHANGED_MODE también valen: el hilo ya tenía COM.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}

#[cfg(windows)]
fn hello_available() -> bool {
    use windows::Security::Credentials::UI::{
        UserConsentVerifier, UserConsentVerifierAvailability,
    };
    init_com();
    match UserConsentVerifier::CheckAvailabilityAsync().and_then(|op| op.get()) {
        Ok(a) => a == UserConsentVerifierAvailability::Available,
        Err(e) => {
            tracing::warn!(target: "lock", "Windows Hello no disponible: {e}");
            false
        }
    }
}

#[cfg(windows)]
fn verify_hello(raw_hwnd: isize, message: &str) -> HelloOutcome {
    use windows::core::{factory, HSTRING};
    use windows::Security::Credentials::UI::{
        UserConsentVerificationResult as R, UserConsentVerifier,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::IUserConsentVerifierInterop;

    init_com();
    let msg = HSTRING::from(message);
    let result = (|| -> windows::core::Result<R> {
        if raw_hwnd != 0 {
            let interop = factory::<UserConsentVerifier, IUserConsentVerifierInterop>()?;
            // El tipo de la operación async se infiere de la rama `else`
            // (misma IAsyncOperation que devuelve la API sin ventana).
            let op = if false {
                UserConsentVerifier::RequestVerificationAsync(&msg)?
            } else {
                unsafe {
                    interop.RequestVerificationForWindowAsync(
                        HWND(raw_hwnd as *mut core::ffi::c_void),
                        &msg,
                    )?
                }
            };
            op.get()
        } else {
            UserConsentVerifier::RequestVerificationAsync(&msg)?.get()
        }
    })();
    match result {
        Ok(R::Verified) => HelloOutcome::Verified,
        Ok(R::Canceled) => HelloOutcome::Canceled,
        Ok(R::RetriesExhausted) => HelloOutcome::RetriesExhausted,
        Ok(R::DeviceNotPresent) | Ok(R::NotConfiguredForUser) | Ok(R::DisabledByPolicy) => {
            HelloOutcome::Unavailable
        }
        Ok(_) => HelloOutcome::Failed,
        Err(e) => {
            tracing::warn!(target: "lock", "Windows Hello falló: {e}");
            HelloOutcome::Failed
        }
    }
}

#[cfg(windows)]
fn verify_password(password: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        LogonUserW, LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT,
    };
    if password.is_empty() {
        return false;
    }
    let user = HSTRING::from(std::env::var("USERNAME").unwrap_or_default());
    let pass = HSTRING::from(password);
    // "." = cuenta local (también las vinculadas a cuenta Microsoft);
    // si no, probamos con el dominio del usuario.
    let mut domains = vec![".".to_string()];
    if let Ok(d) = std::env::var("USERDOMAIN") {
        if !d.trim().is_empty() {
            domains.push(d);
        }
    }
    for d in domains {
        let dom = HSTRING::from(d);
        let mut token = HANDLE::default();
        let ok = unsafe {
            LogonUserW(
                &user,
                &dom,
                &pass,
                LOGON32_LOGON_NETWORK,
                LOGON32_PROVIDER_DEFAULT,
                &mut token,
            )
        }
        .is_ok();
        if ok {
            let _ = unsafe { CloseHandle(token) };
            return true;
        }
    }
    false
}

#[cfg(not(windows))]
fn hello_available() -> bool {
    false
}

#[cfg(not(windows))]
fn verify_hello(_raw_hwnd: isize, _message: &str) -> HelloOutcome {
    HelloOutcome::Unavailable
}

#[cfg(not(windows))]
fn verify_password(_password: &str) -> bool {
    true
}
