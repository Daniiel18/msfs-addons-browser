//! Sincronización con Google Drive (v2.0.0).
//!
//! ## Flujo
//!
//! 1. **Credentials**: el usuario pega su Client ID + Client Secret de
//!    un OAuth client tipo "Desktop app" creado en Google Cloud Console.
//!    No podemos shippearlas porque Google no permite distribuir
//!    credenciales de un proyecto sin verificación — el usuario crea
//!    sus propias (instrucciones en la UI).
//! 2. **Auth start**: generamos un PKCE code_verifier + code_challenge,
//!    abrimos un listener TCP en `127.0.0.1:PORT_ALEATORIO` y
//!    devolvemos al frontend el URL de autorización de Google. El
//!    usuario lo abre en su navegador.
//! 3. **Callback**: Google redirige a `http://127.0.0.1:PORT/?code=...`.
//!    El listener parsea el code, lo cambia por tokens en el endpoint
//!    `/token`, guarda el refresh_token + email en `settings` y emite
//!    el evento `cloud://oauth-completed` al frontend con éxito o
//!    error.
//! 4. **Sync**: con un access_token fresco (refrescado on-demand),
//!    leemos/escribimos en el folder privado `appDataFolder` de
//!    Drive — invisible para el usuario y aislado por aplicación.
//!
//! ## Lo que sincronizamos
//!
//! Una sola "snapshot" como JSON en `msfs-addons-data.json`:
//!   · `flight_log` (vuelos reales).
//!   · `flight_log_track` (puntos de track).
//!   · `settings` con prefijo `pref_`.
//!
//! Tras formatear PC + reinstalar app + login en Google, recuperas
//! tu bitácora y preferencias.
//!
//! ## Conflict resolution
//!
//! Última escritura gana. Es deliberado (cada usuario suele tener un
//! solo PC corriendo MSFS a la vez); no nos complicamos con merges
//! por timestamp por fila.

use std::time::Duration;

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;


// =============================================================================
// (v4.14.0 #2) Progreso de transferencia cloud — barra + velocidad + ETA.
//
// Emitimos `cloud://progress` mientras se sube/baja el snapshot. La
// velocidad (MB/s) y el ETA los calcula el frontend a partir de los deltas
// de bytes entre eventos consecutivos, así el backend solo reporta bytes.
//
// IMPORTANTE: el byte-count refleja el throughput de NUESTRA transferencia
// (bytes entregados al socket en subida / leídos de la respuesta en bajada),
// no el procesamiento interno de Google — pero da %, MB/s y ETA correctos.
// =============================================================================

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferProgress {
    /// "upload" | "download".
    phase: &'static str,
    /// Bytes transferidos hasta ahora (comprimidos / como van por el cable).
    transferred: u64,
    /// Tamaño total en bytes. 0 = desconocido (bajada sin Content-Length).
    total: u64,
    /// true en el último evento de la fase.
    done: bool,
}

/// Emite un evento de progreso si hay un `AppHandle` (las rutas sin UI —
/// auto-sync diario, purge — pasan `None` y no emiten nada).
fn emit_transfer_progress(
    app: Option<&AppHandle>,
    phase: &'static str,
    transferred: u64,
    total: u64,
    done: bool,
) {
    if let Some(app) = app {
        let _ = app.emit(
            "cloud://progress",
            TransferProgress {
                phase,
                transferred,
                total,
                done,
            },
        );
    }
}
const SCOPE: &str = "https://www.googleapis.com/auth/drive.appdata https://www.googleapis.com/auth/userinfo.email";

// (v3.1.0 / v3.1.1) Credenciales OAuth embebidas en el binario al
// compilar. NO van al git por dos razones:
//   1. GitHub secret scanning bloquea el push si detecta un Google
//      OAuth secret en plain text.
//   2. Aunque el repo esté privado hoy, podría volverse público y
//      filtrar el secret.
//
// El flujo es:
//   · `build.rs` lee `secrets.local.toml` (gitignored) si existe y
//     re-exporta los valores como `cargo:rustc-env=…`.
//   · `option_env!` los inserta como `Some("...")` en el binario.
//   · Si el archivo no existe (e.g. clonando fresh sin acceso al
//     secret), las consts caen a `None` y el flujo OAuth se rechaza
//     con un mensaje claro pidiendo configurar el archivo.
const HARDCODED_CLIENT_ID: Option<&str> =
    option_env!("SIMFLEET_GOOGLE_CLIENT_ID");
const HARDCODED_CLIENT_SECRET: Option<&str> =
    option_env!("SIMFLEET_GOOGLE_CLIENT_SECRET");

// (v7.4.0) La allowlist de correos autorizados a sync ya NO vive en claro
// aquí (el repo es público): sus hashes SHA-256 están en `crate::owner_ids`.
// La comparación por hash preserva el mismo comportamiento sin exponer las
// direcciones. Añadir una cuenta = añadir su hash allí y recompilar.

const KEY_CLIENT_ID: &str = "google_client_id";
const KEY_CLIENT_SECRET: &str = "google_client_secret";
const KEY_REFRESH_TOKEN: &str = "google_refresh_token";
const KEY_USER_EMAIL: &str = "google_user_email";
const KEY_LAST_SYNC_AT: &str = "google_last_sync_at";
/// (v7.4.2) Marca de "Google revocó/expiró el refresh token" (`invalid_grant`).
/// Mientras exista, la UI pide reconectar y los auto-uploads no reintentan.
const KEY_SESSION_EXPIRED: &str = "google_session_expired";

/// (v3.1.0) Devuelve `(client_id, client_secret)` consultando primero
/// los valores hardcoded de build, luego DB como fallback (compat con
/// users en v2.x que configuraron via UI).
async fn resolve_credentials(
    pool: &SqlitePool,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    if let (Some(cid), Some(secret)) =
        (HARDCODED_CLIENT_ID, HARDCODED_CLIENT_SECRET)
    {
        if !cid.is_empty() && !secret.is_empty() {
            return Ok((Some(cid.to_string()), Some(secret.to_string())));
        }
    }
    let cid = get_setting(pool, KEY_CLIENT_ID).await?;
    let secret = get_setting(pool, KEY_CLIENT_SECRET).await?;
    Ok((cid, secret))
}

/// (v3.1.0) Verifica que el email esté autorizado a sync. La lista de
/// cuentas vive hasheada (SHA-256) en `crate::owner_ids` porque el repo
/// es público — comparar hashes es equivalente a comparar los correos.
fn is_whitelisted(email: &str) -> bool {
    crate::owner_ids::is_sync_authorized(email)
}

/// (v2.0.1) Folder sync — alternativa simple a OAuth. Apunta a una
/// carpeta de OneDrive/Google Drive Desktop/Dropbox/iCloud y el
/// cliente de esa nube se encarga de subir/bajar el JSON. La app
/// recuerda este path para repetir el sync sin re-elegir cada vez.
const KEY_FOLDER_SYNC_PATH: &str = "cloud_folder_sync_path";
const KEY_FOLDER_SYNC_LAST_AT: &str = "cloud_folder_sync_last_at";

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudConfig {
    pub connected: bool,
    pub has_credentials: bool,
    /// (v7.4.2) Google rechazó el refresh token (`invalid_grant`: revocado,
    /// contraseña cambiada o expirado — p.ej. app OAuth en modo "Testing",
    /// cuyos tokens caducan a los 7 días). Hay que volver a Conectar.
    pub session_expired: bool,
    pub user_email: Option<String>,
    pub last_sync_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OauthStart {
    pub auth_url: String,
    pub redirect_uri: String,
    pub port: u16,
}

/// Evento emitido al frontend al terminar el flow OAuth.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OauthCompletedEvent {
    pub ok: bool,
    pub user_email: Option<String>,
    pub error: Option<String>,
}

// =============================================================================
// Settings helpers
// =============================================================================

async fn get_setting(pool: &SqlitePool, key: &str) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = ?1")
            .bind(key)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0))
}

async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> anyhow::Result<()> {
    sqlx::query(
        r#"INSERT INTO settings (key, value, updated_at)
           VALUES (?1, ?2, datetime('now'))
           ON CONFLICT(key) DO UPDATE SET
               value = excluded.value,
               updated_at = excluded.updated_at"#,
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

async fn delete_setting(pool: &SqlitePool, key: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM settings WHERE key = ?1")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

// =============================================================================
// Public API
// =============================================================================

pub async fn get_config(pool: &SqlitePool) -> anyhow::Result<CloudConfig> {
    let (client_id, client_secret) = resolve_credentials(pool).await?;
    let refresh_token = get_setting(pool, KEY_REFRESH_TOKEN).await?;
    let user_email = get_setting(pool, KEY_USER_EMAIL).await?;
    let last_sync_at = get_setting(pool, KEY_LAST_SYNC_AT).await?;
    let has_credentials = client_id.as_deref().filter(|s| !s.is_empty()).is_some()
        && client_secret.as_deref().filter(|s| !s.is_empty()).is_some();
    let connected = has_credentials
        && refresh_token.as_deref().filter(|s| !s.is_empty()).is_some();
    let session_expired = get_setting(pool, KEY_SESSION_EXPIRED).await?.is_some();
    Ok(CloudConfig {
        connected,
        has_credentials,
        session_expired,
        user_email,
        last_sync_at,
    })
}

pub async fn set_credentials(
    pool: &SqlitePool,
    client_id: &str,
    client_secret: &str,
) -> anyhow::Result<()> {
    set_setting(pool, KEY_CLIENT_ID, client_id.trim()).await?;
    set_setting(pool, KEY_CLIENT_SECRET, client_secret.trim()).await?;
    Ok(())
}

pub async fn disconnect(pool: &SqlitePool) -> anyhow::Result<()> {
    delete_setting(pool, KEY_REFRESH_TOKEN).await?;
    delete_setting(pool, KEY_USER_EMAIL).await?;
    delete_setting(pool, KEY_LAST_SYNC_AT).await?;
    delete_setting(pool, KEY_SESSION_EXPIRED).await?;
    Ok(())
}

// =============================================================================
// OAuth flow — listener task que hace TODO el ciclo
// =============================================================================

pub async fn start_oauth(
    pool: SqlitePool,
    http: reqwest::Client,
    app: AppHandle,
) -> anyhow::Result<OauthStart> {
    // (v3.1.0) Credenciales desde build env > DB fallback.
    let (client_id_opt, client_secret_opt) = resolve_credentials(&pool).await?;
    let client_id = client_id_opt
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("{}", crate::tr!("cloudSync.credsNotEmbedded")))?;
    let client_secret = client_secret_opt
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("{}", crate::tr!("cloudSync.credsNotEmbeddedShort")))?;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context(crate::tr!("cloudSync.listenerOpenFailed"))?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{}/", port);

    let code_verifier = random_url_safe_string(64);
    let code_challenge = base64_urlsafe_no_pad(&sha256(code_verifier.as_bytes()));
    let state = random_url_safe_string(32);

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?\
         client_id={cid}&\
         redirect_uri={redir}&\
         response_type=code&\
         scope={scope}&\
         code_challenge={chal}&\
         code_challenge_method=S256&\
         state={state}&\
         access_type=offline&\
         prompt=consent",
        cid = urlencoding::encode(&client_id),
        redir = urlencoding::encode(&redirect_uri),
        scope = urlencoding::encode(SCOPE),
        chal = urlencoding::encode(&code_challenge),
        state = urlencoding::encode(&state),
    );

    // Spawn de la tarea que recibe el callback y completa el flow.
    let redirect_for_task = redirect_uri.clone();
    tokio::spawn(async move {
        let result = handle_callback(
            listener,
            state,
            code_verifier,
            client_id,
            client_secret,
            redirect_for_task,
            pool.clone(),
            http.clone(),
        )
        .await;
        let payload = match result {
            Ok(email) => OauthCompletedEvent {
                ok: true,
                user_email: email,
                error: None,
            },
            Err(e) => {
                tracing::warn!(target: "cloud", "OAuth fallo: {e:#}");
                OauthCompletedEvent {
                    ok: false,
                    user_email: None,
                    error: Some(e.to_string()),
                }
            }
        };
        let _ = app.emit("cloud://oauth-completed", payload);
    });

    Ok(OauthStart {
        auth_url,
        redirect_uri,
        port,
    })
}

async fn handle_callback(
    listener: TcpListener,
    expected_state: String,
    code_verifier: String,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    pool: SqlitePool,
    http: reqwest::Client,
) -> anyhow::Result<Option<String>> {
    // 5 min timeout para que el usuario termine en el navegador.
    let timeout = tokio::time::sleep(Duration::from_secs(300));
    tokio::pin!(timeout);

    let (mut socket, _peer) = tokio::select! {
        r = listener.accept() => r?,
        _ = &mut timeout => return Err(anyhow!("{}", crate::tr!("cloudSync.authTimeout"))),
    };

    let mut buf = [0u8; 8192];
    let mut total = 0usize;
    loop {
        let n = socket.read(&mut buf[total..]).await?;
        if n == 0 {
            break;
        }
        total += n;
        if buf[..total].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if total >= buf.len() {
            break;
        }
    }
    let req = String::from_utf8_lossy(&buf[..total]).to_string();
    let line1 = req.lines().next().unwrap_or("");
    let path = line1.split_whitespace().nth(1).unwrap_or("");
    let (code, state) = parse_callback_query(path);

    let success = state.as_deref() == Some(&expected_state) && code.is_some();
    let body = if !success {
        format!(
            "<!doctype html><html><body style='font-family:system-ui;padding:32px;background:#0f172a;color:#e2e8f0'><h2>❌ {title}</h2><p>{msg}</p></body></html>",
            title = crate::tr!("cloudSync.oauthPageErrorTitle"),
            msg = crate::tr!("cloudSync.oauthPageErrorBody"),
        )
    } else {
        format!(
            "<!doctype html><html><body style='font-family:system-ui;padding:32px;background:#0f172a;color:#e2e8f0'><h2>✓ {title}</h2><p>{msg}</p></body></html>",
            title = crate::tr!("cloudSync.oauthPageOkTitle"),
            msg = crate::tr!("cloudSync.oauthPageOkBody"),
        )
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;

    if !success {
        return Err(anyhow!("{}", crate::tr!("cloudSync.callbackInvalid")));
    }
    let code = code.unwrap();

    let tokens = exchange_code_for_tokens(
        &http,
        &client_id,
        &client_secret,
        &code,
        &code_verifier,
        &redirect_uri,
    )
    .await?;

    let refresh_token = tokens.refresh_token.ok_or_else(|| {
        anyhow!("{}", crate::tr!("cloudSync.noRefreshToken"))
    })?;

    let email = fetch_user_email(&http, &tokens.access_token).await.ok();

    // (v3.1.0) Whitelist check — sólo emails aprobados pueden
    // completar el OAuth. Si el email no está en la lista, descartamos
    // refresh_token y devolvemos error claro al usuario.
    if let Some(ref e) = email {
        if !is_whitelisted(e) {
            tracing::warn!(
                target: "cloud",
                "OAuth completado pero email '{}' no está en WHITELIST_EMAILS — rechazando",
                e
            );
            return Err(anyhow!("{}", crate::tr!("cloudSync.emailNotAuthorized", email = e)));
        }
    } else {
        return Err(anyhow!("{}", crate::tr!("cloudSync.emailFetchFailed")));
    }

    set_setting(&pool, KEY_REFRESH_TOKEN, &refresh_token).await?;
    if let Some(e) = &email {
        set_setting(&pool, KEY_USER_EMAIL, e).await?;
    }
    delete_setting(&pool, KEY_SESSION_EXPIRED).await?;
    Ok(email)
}

fn parse_callback_query(path: &str) -> (Option<String>, Option<String>) {
    let q = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    for pair in q.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let decoded = urlencoding::decode(v).map(|s| s.into_owned()).unwrap_or_default();
            match k {
                "code" => code = Some(decoded),
                "state" => state = Some(decoded),
                _ => {}
            }
        }
    }
    (code, state)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

async fn exchange_code_for_tokens(
    http: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> anyhow::Result<TokenResponse> {
    let resp = http
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("code_verifier", code_verifier),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        if token_error_code(&txt).as_deref() == Some("invalid_client") {
            return Err(anyhow!("{}", crate::tr!("cloudSync.invalidClient")));
        }
        return Err(anyhow!("{}", crate::tr!("cloudSync.tokenEndpointError", status = status, body = truncate_body(&txt))));
    }
    Ok(resp.json().await?)
}

#[derive(Deserialize)]
struct UserInfo {
    email: String,
}

async fn fetch_user_email(http: &reqwest::Client, access_token: &str) -> anyhow::Result<String> {
    let resp = http
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .bearer_auth(access_token)
        .send()
        .await?
        .error_for_status()?;
    let info: UserInfo = resp.json().await?;
    Ok(info.email)
}

async fn fresh_access_token(
    pool: &SqlitePool,
    http: &reqwest::Client,
) -> anyhow::Result<String> {
    let refresh_token = get_setting(pool, KEY_REFRESH_TOKEN)
        .await?
        .ok_or_else(|| anyhow!("{}", crate::tr!("cloudSync.notConnected")))?;
    // (v3.6.3 fix I13) Usar resolve_credentials para que mire los valores
    // hardcoded del binario primero (build.rs + secrets.local.toml) y
    // SOLO si faltan caiga al fallback DB. Antes leía solo DB y daba
    // "Client ID no configurado" en step 3 del diagnostic aunque las
    // credenciales estuvieran embedidas correctamente.
    let (cid_opt, secret_opt) = resolve_credentials(pool).await?;
    let client_id = cid_opt
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("{}", crate::tr!("cloudSync.clientIdNotEmbedded")))?;
    let client_secret = secret_opt
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("{}", crate::tr!("cloudSync.clientSecretNotEmbedded")))?;
    let resp = http
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        // (v7.4.2) `invalid_grant` = el refresh token ya no sirve y reintentar
        // no lo arregla. Antes quedaba guardado → la UI seguía mostrando
        // "Conectado" y CADA vuelo (auto-upload) y cada auto-sync diario
        // fallaban con el JSON crudo de Google en un toast. Ahora lo
        // descartamos (conservando el email para el aviso) y marcamos la
        // sesión como expirada: la UI pide reconectar y nadie reintenta.
        // (v7.4.2) `invalid_client` = el client secret embebido ya no vale
        // (rotado en Google Cloud). No es culpa de la sesión: no la tocamos.
        if token_error_code(&txt).as_deref() == Some("invalid_client") {
            return Err(anyhow!("{}", crate::tr!("cloudSync.invalidClient")));
        }
        if is_invalid_grant(&txt) {
            tracing::warn!(
                target: "cloud",
                "refresh token rechazado (invalid_grant) — sesión de Google marcada como expirada"
            );
            delete_setting(pool, KEY_REFRESH_TOKEN).await?;
            set_setting(pool, KEY_SESSION_EXPIRED, "1").await?;
            return Err(anyhow!("{}", crate::tr!("cloudSync.sessionExpired")));
        }
        return Err(anyhow!("{}", crate::tr!("cloudSync.refreshTokenRejected", status = status, body = truncate_body(&txt))));
    }
    let body: TokenResponse = resp.json().await?;
    Ok(body.access_token)
}

// =============================================================================
// Drive API
// =============================================================================

#[derive(Deserialize)]
struct DriveFile {
    id: String,
}

#[derive(Deserialize)]
struct DriveList {
    #[serde(default)]
    files: Vec<DriveFile>,
}

/// (v2.0.1 · v3.38.0 #12) Convierte un error HTTP de Drive (CON
/// respuesta) en un mensaje accionable, clasificando por código y
/// recortando el cuerpo de Google (a veces HTML/JSON largo).
async fn drive_error_for_response(resp: reqwest::Response) -> anyhow::Error {
    let status = resp.status();
    let code = status.as_u16();
    let body = truncate_body(&resp.text().await.unwrap_or_default());
    match code {
        401 => anyhow!("{}", crate::tr!("cloudSync.drive401", body = body)),
        403 => anyhow!("{}", crate::tr!("cloudSync.drive403", body = body)),
        404 => anyhow!("{}", crate::tr!("cloudSync.drive404", body = body)),
        429 => anyhow!("{}", crate::tr!("cloudSync.drive429", body = body)),
        500..=599 => anyhow!("{}", crate::tr!("cloudSync.drive5xx", code = code, body = body)),
        _ => anyhow!("Google Drive {status}: {body}"),
    }
}

/// (v3.38.0 #12) Clasifica un error de TRANSPORTE de reqwest (ocurre
/// ANTES de recibir una respuesta HTTP): timeout, conexión/DNS, TLS…
/// en un mensaje accionable. Para errores CON respuesta se usa
/// `drive_error_for_response`.
fn transport_error(context: &str, e: reqwest::Error) -> anyhow::Error {
    let kind = if e.is_timeout() {
        crate::tr!("cloudSync.transportTimeout")
    } else if e.is_connect() {
        crate::tr!("cloudSync.transportConnect")
    } else if e.is_request() {
        crate::tr!("cloudSync.transportRequest")
    } else if e.is_body() || e.is_decode() {
        crate::tr!("cloudSync.transportDecode")
    } else {
        crate::tr!("cloudSync.transportGeneric")
    };
    anyhow!("{}", crate::tr!("cloudSync.transportWrap", context = context, kind = kind, e = e))
}

/// (v3.38.0 #12) Acorta un fileId de Drive para logs/errores — no es
/// secreto pero es ruido largo; mostramos los primeros 8 caracteres.
fn sanitize_id(id: &str) -> String {
    let head: String = id.chars().take(8).collect();
    if id.chars().count() > 8 {
        format!("{head}…")
    } else {
        head
    }
}

/// (v7.4.2) Código `error` de una respuesta del endpoint de tokens de Google
/// (`invalid_grant`, `invalid_client`…), si el cuerpo es JSON.
fn token_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("error")?
        .as_str()
        .map(str::to_string)
}

/// (v7.4.2) ¿La respuesta del endpoint de tokens es un `invalid_grant`?
fn is_invalid_grant(body: &str) -> bool {
    match token_error_code(body) {
        Some(code) => code == "invalid_grant",
        None => body.contains("invalid_grant"),
    }
}

/// (v3.38.0 #12) Recorta el cuerpo de error de Google a algo legible.
fn truncate_body(body: &str) -> String {
    let trimmed = body.trim();
    const MAX: usize = 300;
    if trimmed.chars().count() > MAX {
        let head: String = trimmed.chars().take(MAX).collect();
        format!("{head}… {suffix}", suffix = crate::tr!("cloudSync.truncatedSuffix"))
    } else {
        trimmed.to_string()
    }
}

async fn find_sync_file(
    http: &reqwest::Client,
    access_token: &str,
    // (v7.4.2) Snapshot a buscar: el de subida o el de bajada (ver `test_env`).
    name: &str,
) -> anyhow::Result<Option<String>> {
    let q = format!("name = '{}' and 'appDataFolder' in parents", name);
    let url = format!(
        "https://www.googleapis.com/drive/v3/files?spaces=appDataFolder&q={}&fields=files(id)",
        urlencoding::encode(&q)
    );
    let resp = http
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| transport_error(&crate::tr!("cloudSync.ctxSearchingSnapshot"), e))?;
    if !resp.status().is_success() {
        return Err(drive_error_for_response(resp).await);
    }
    let list: DriveList = resp.json().await?;
    Ok(list.files.into_iter().next().map(|f| f.id))
}

/// Timeout generoso para las transferencias del snapshot a/desde Drive.
/// La DB de tracks puede pesar cientos de MB; aunque la comprimimos con
/// gzip, una subida lenta no debe morir por el timeout corto del cliente
/// global. El usuario reportó "le di a upload y no hace nada" — la causa
/// era exactamente esto (operation timed out en el log).
const DRIVE_TRANSFER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Comprime bytes con gzip (nivel por defecto). El snapshot JSON
/// comprime ~15-20x — convierte cientos de MB en decenas, que sí caben
/// en una subida con timeout razonable.
fn gzip_bytes(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data)?;
    Ok(enc.finish()?)
}

/// Descomprime si los bytes son gzip (magic `1f 8b`); si no, los trata
/// como texto plano UTF-8. Esto da compatibilidad hacia atrás con los
/// snapshots VIEJOS que se subieron sin comprimir.
fn maybe_gunzip_to_string(bytes: Vec<u8>) -> anyhow::Result<String> {
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let mut dec = GzDecoder::new(&bytes[..]);
        let mut out = String::new();
        dec.read_to_string(&mut out)?;
        Ok(out)
    } else {
        Ok(String::from_utf8(bytes)?)
    }
}

async fn create_sync_file(
    http: &reqwest::Client,
    access_token: &str,
    content: &str,
    app: Option<&AppHandle>,
) -> anyhow::Result<String> {
    let metadata = serde_json::json!({
        "name": crate::test_env::sync_upload_file_name(),
        "parents": ["appDataFolder"],
    });
    // (v4.0.1) Subimos el payload COMPRIMIDO con gzip. Construimos el
    // cuerpo multipart como bytes (no `format!`) porque la parte del
    // archivo es binaria.
    let gz = gzip_bytes(content.as_bytes())?;
    let boundary = "msfs_addons_boundary_42";
    let mut body: Vec<u8> = Vec::with_capacity(gz.len() + 256);
    body.extend_from_slice(
        format!(
            "--{b}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{b}\r\nContent-Type: application/gzip\r\n\r\n",
            b = boundary,
            meta = metadata,
        )
        .as_bytes(),
    );
    body.extend_from_slice(&gz);
    body.extend_from_slice(format!("\r\n--{b}--", b = boundary).as_bytes());
    let total = body.len() as u64;
    let resp = http
        .post("https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart")
        .bearer_auth(access_token)
        .header(
            "Content-Type",
            format!("multipart/related; boundary={}", boundary),
        )
        .timeout(DRIVE_TRANSFER_TIMEOUT)
        .body(body)
        .send()
        .await
        .map_err(|e| transport_error(&crate::tr!("cloudSync.ctxCreatingSnapshot"), e))?;
    if !resp.status().is_success() {
        return Err(drive_error_for_response(resp).await);
    }
    // (v4.14.1 #2) Un único evento final con el TAMAÑO real subido — el
    // frontend deriva MB/s = bytes / duración y anima la barra ~1.5s.
    emit_transfer_progress(app, "upload", total, total, true);
    crate::diagnostics::note_cloud_upload(total);
    let file: DriveFile = resp.json().await?;
    crate::success!(
        target: "cloud_sync",
        "Snapshot creado en Drive (file {})",
        sanitize_id(&file.id)
    );
    Ok(file.id)
}

async fn update_sync_file(
    http: &reqwest::Client,
    access_token: &str,
    file_id: &str,
    content: &str,
    app: Option<&AppHandle>,
) -> anyhow::Result<()> {
    // (v4.0.1) Payload comprimido con gzip + timeout largo. Antes se
    // subía el JSON crudo (cientos de MB) y la request moría por
    // "operation timed out".
    let gz = gzip_bytes(content.as_bytes())?;
    let total = gz.len() as u64;
    let resp = http
        .patch(format!(
            "https://www.googleapis.com/upload/drive/v3/files/{}?uploadType=media",
            file_id
        ))
        .bearer_auth(access_token)
        .header("Content-Type", "application/gzip")
        .timeout(DRIVE_TRANSFER_TIMEOUT)
        .body(gz)
        .send()
        .await
        .map_err(|e| transport_error(&crate::tr!("cloudSync.ctxUpdatingSnapshot"), e))?;
    if !resp.status().is_success() {
        return Err(drive_error_for_response(resp).await);
    }
    // (v4.14.1 #2) Evento final con el tamaño subido (gz) para MB/s real.
    emit_transfer_progress(app, "upload", total, total, true);
    crate::diagnostics::note_cloud_upload(total);
    crate::success!(
        target: "cloud_sync",
        "Snapshot actualizado en Drive (file {})",
        sanitize_id(file_id)
    );
    Ok(())
}

async fn download_sync_file(
    http: &reqwest::Client,
    access_token: &str,
    app: Option<&AppHandle>,
    name: &str,
) -> anyhow::Result<Option<String>> {
    let Some(file_id) = find_sync_file(http, access_token, name).await? else {
        return Ok(None);
    };
    let resp = http
        .get(format!(
            "https://www.googleapis.com/drive/v3/files/{}?alt=media",
            file_id
        ))
        .bearer_auth(access_token)
        .timeout(DRIVE_TRANSFER_TIMEOUT)
        .send()
        .await
        .map_err(|e| transport_error(&crate::tr!("cloudSync.ctxDownloadingSnapshot"), e))?;
    if !resp.status().is_success() {
        return Err(drive_error_for_response(resp).await);
    }
    // (v4.0.1) El snapshot puede venir gzipeado (nuevo) o en texto plano
    // (snapshots viejos). `maybe_gunzip_to_string` detecta por magic bytes.
    let bytes = resp.bytes().await?.to_vec();
    // (v4.14.1 #2) Evento final con el tamaño bajado (comprimido) para
    // que el frontend muestre MB/s real.
    emit_transfer_progress(app, "download", bytes.len() as u64, bytes.len() as u64, true);
    Ok(Some(maybe_gunzip_to_string(bytes)?))
}

/// Borra el snapshot file de Drive y empuja un snapshot VACÍO en su
/// lugar para que la próxima ronda de sync no resucite el historial
/// desde un caché en otra máquina. Devuelve `(deleted_flights,
/// deleted_tracks)` — los conteos vistos en el snapshot remoto antes
/// de borrarlo, así el usuario sabe qué se perdió.
///
/// **Importante**: NO toca las tablas locales — los vuelos del usuario
/// en este PC se preservan. Sólo limpia la copia cloud.
async fn purge_cloud_snapshot(
    http: &reqwest::Client,
    access_token: &str,
) -> anyhow::Result<PurgeReport> {
    let mut report = PurgeReport::default();
    // Leer el snapshot remoto para reportar lo que se borra.
    if let Some(text) = download_sync_file(http, access_token, None, crate::test_env::sync_upload_file_name()).await? {
        if let Ok(snap) = serde_json::from_str::<Snapshot>(&text) {
            report.deleted_flights = snap.flight_log.len();
            report.deleted_tracks = snap.flight_log_track.len();
            report.deleted_settings = snap.settings.len();
        }
    }
    // Borrar el archivo de Drive si existe.
    if let Some(file_id) = find_sync_file(http, access_token, crate::test_env::sync_upload_file_name()).await? {
        let resp = http
            .delete(format!(
                "https://www.googleapis.com/drive/v3/files/{}",
                file_id
            ))
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| transport_error(&crate::tr!("cloudSync.ctxDeletingSnapshot"), e))?;
        if !resp.status().is_success() {
            return Err(drive_error_for_response(resp).await);
        }
        report.file_deleted = true;
    }
    Ok(report)
}

/// Reporte de la operación de purga cloud.
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeReport {
    pub deleted_flights: usize,
    pub deleted_tracks: usize,
    pub deleted_settings: usize,
    /// True si encontramos y borramos el snapshot file de Drive.
    /// False si no había snapshot (purga es no-op exitosa).
    pub file_deleted: bool,
}

/// API pública de purga — wrap del helper interno con manejo de auth.
/// (v3.5.0) Tras purgar, también resetea el setting `last_sync_at`
/// para que la UI no muestre un timestamp viejo.
pub async fn purge(
    pool: &SqlitePool,
    http: &reqwest::Client,
) -> anyhow::Result<PurgeReport> {
    let access_token = fresh_access_token(pool, http).await?;
    let report = purge_cloud_snapshot(http, &access_token).await?;
    // Reset del last_sync_at para que la UI refleje "nunca sincronizado".
    // (v4.19.0) Fix: la tabla se llama `settings`, no `app_settings` —
    // el typo hacía fallar TODO el purge con "no such table" justo
    // después de borrar el snapshot de Drive.
    sqlx::query("DELETE FROM settings WHERE key = ?1")
        .bind(KEY_LAST_SYNC_AT)
        .execute(pool)
        .await?;
    tracing::info!(
        target: "cloud",
        "cloud purged · {} flights · {} tracks · {} settings · file_deleted={}",
        report.deleted_flights,
        report.deleted_tracks,
        report.deleted_settings,
        report.file_deleted,
    );
    Ok(report)
}

// =============================================================================
// Snapshot model
// =============================================================================

/// (24/10) v3: + libro (`ledger`) y borrados (`deleted`).
const SNAPSHOT_VERSION: u32 = 3;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    version: u32,
    exported_at: String,
    flight_log: Vec<serde_json::Value>,
    flight_log_track: Vec<serde_json::Value>,
    settings: Vec<SettingKV>,
    /// (v3.6.0 Phase H — Epic E) Breakdown del scoring rule-by-rule.
    /// Se sincroniza con el cloud para que el usuario pueda ver el
    /// checklist desde otra máquina sin recomputar.
    #[serde(default)]
    flight_log_score_item: Vec<serde_json::Value>,
    /// (v3.6.0) Registro de transiciones de fase por vuelo. Necesario
    /// para que el re-scoring funcione desde cualquier máquina.
    #[serde(default)]
    flight_log_phase: Vec<serde_json::Value>,
    /// (24/10) Libro de mantenimiento y finanzas (Tech Log de SimFleet,
    /// cobros, líneas de GSX, políticas, traslados…) — `ledger_sync`.
    #[serde(default)]
    ledger: crate::ledger_sync::Ledger,
    /// (24/10) Lo borrado a propósito: no vuelve y se borra en las demás PCs.
    #[serde(default)]
    deleted: Vec<crate::ledger_sync::Tombstone>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct SettingKV {
    key: String,
    value: String,
}

/// (v2.0.3) Construye la snapshot leyendo el schema EN RUNTIME via
/// `SELECT *`. Antes hardcodeábamos los nombres de columnas en un
/// `json_object(...)` SQL, lo que se rompía si una migración cambiaba
/// un nombre (caso real: `last_position_ground_speed_kt` no existía,
/// la columna real es `last_position_gs_kt`).
///
/// Ahora iteramos las columnas devueltas por SQLite y serializamos
/// cada valor según su tipo. Las claves se convierten a camelCase
/// para que el JSON sea consumible por el frontend sin cambios.
async fn build_snapshot(pool: &SqlitePool) -> anyhow::Result<Snapshot> {
    let flight_rows = sqlx::query("SELECT * FROM flight_log")
        .fetch_all(pool)
        .await?;
    let track_rows =
        sqlx::query("SELECT * FROM flight_log_track ORDER BY flight_id, ts")
            .fetch_all(pool)
            .await?;
    // (v3.6.0 Phase H) Snapshots de scoring (rule breakdown + phase
    // transitions). El ORDER BY mantiene snapshots determinísticos —
    // útil para diff/auditoria de la nube.
    let score_rows =
        sqlx::query("SELECT * FROM flight_log_score_item ORDER BY flight_id, phase, rule_id")
            .fetch_all(pool)
            .await?;
    let phase_rows =
        sqlx::query("SELECT * FROM flight_log_phase ORDER BY flight_id, entered_at")
            .fetch_all(pool)
            .await?;
    let settings_rows: Vec<SettingKV> = sqlx::query_as(
        "SELECT key, value FROM settings WHERE key LIKE 'pref_%'",
    )
    .fetch_all(pool)
    .await?;
    Ok(Snapshot {
        version: SNAPSHOT_VERSION,
        exported_at: chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string(),
        flight_log: flight_rows.iter().map(row_to_json).collect(),
        flight_log_track: track_rows.iter().map(row_to_json).collect(),
        settings: settings_rows,
        flight_log_score_item: score_rows.iter().map(row_to_json).collect(),
        flight_log_phase: phase_rows.iter().map(row_to_json).collect(),
        ledger: crate::ledger_sync::export(pool).await?,
        deleted: crate::ledger_sync::load_tombstones(pool).await?,
    })
}

/// Convierte una fila genérica de SQLite a un `serde_json::Value`
/// objeto, con claves en camelCase y valores tipados según el `type
/// info` de SQLite (INTEGER → number, REAL → number, TEXT → string,
/// NULL → null).
pub(crate) fn row_to_json(row: &sqlx::sqlite::SqliteRow) -> serde_json::Value {
    use sqlx::{Column, Row, TypeInfo, ValueRef};
    let mut obj = serde_json::Map::new();
    for (i, col) in row.columns().iter().enumerate() {
        let snake = col.name();
        let key = snake_to_camel(snake);
        let raw = match row.try_get_raw(i) {
            Ok(r) => r,
            Err(_) => {
                obj.insert(key, serde_json::Value::Null);
                continue;
            }
        };
        if raw.is_null() {
            obj.insert(key, serde_json::Value::Null);
            continue;
        }
        let type_name = raw.type_info().name().to_ascii_uppercase();
        let value: serde_json::Value = match type_name.as_str() {
            "INTEGER" | "INT" | "BIGINT" => row
                .try_get::<i64, _>(i)
                .ok()
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
            "REAL" | "FLOAT" | "DOUBLE" => row
                .try_get::<f64, _>(i)
                .ok()
                .and_then(|n| {
                    serde_json::Number::from_f64(n).map(serde_json::Value::Number)
                })
                .unwrap_or(serde_json::Value::Null),
            // BLOB y otros se serializan como base64 si se necesita; por
            // ahora no usamos blobs en flight_log.
            "BLOB" => serde_json::Value::Null,
            // TEXT y cualquier cosa desconocida → string.
            _ => row
                .try_get::<String, _>(i)
                .ok()
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        };
        obj.insert(key, value);
    }
    serde_json::Value::Object(obj)
}

fn snake_to_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper_next = false;
    for c in s.chars() {
        if c == '_' {
            upper_next = true;
            continue;
        }
        if upper_next {
            out.push(c.to_ascii_uppercase());
            upper_next = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// (v7.4.2) Upsert POR ID de un vuelo del snapshot (antes inline en
/// `restore_snapshot`). Con `with_number = false` entra sin
/// flight_number/airline_icao para no chocar con
/// `uq_flight_log_va_dedup_completed` (el callsign se conserva).
fn upsert_snapshot_flight<'q>(
    row: &'q serde_json::Value,
    id: i64,
    started_at: &'q str,
    with_number: bool,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    let (flight_number, airline_icao) = if with_number {
        (json_str(row, "flightNumber"), json_str(row, "airlineIcao"))
    } else {
        (None, None)
    };
    sqlx::query(
        r#"INSERT INTO flight_log
            (id, started_at, ended_at, origin_icao, origin_name, origin_lat, origin_lon,
             destination_icao, destination_name, destination_lat, destination_lon,
             aircraft_atc_type, aircraft_title,
             aircraft_model, aircraft_airline, aircraft_registration,
             distance_nm, flight_time_s,
             max_altitude_ft, landing_fpm, max_ground_speed_kt, max_true_airspeed_kt,
             departure_gate, arrival_gate, passengers, cargo_kg, fuel_used_kg,
             paused_seconds,
             flight_number, callsign, airline_icao, status,
             score_total, score_max, score_grade,
             metar_origin, metar_dest)
           VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
           ON CONFLICT(id) DO UPDATE SET
             started_at = excluded.started_at,
             ended_at = excluded.ended_at,
             origin_icao = excluded.origin_icao,
             origin_name = excluded.origin_name,
             origin_lat = excluded.origin_lat,
             origin_lon = excluded.origin_lon,
             destination_icao = excluded.destination_icao,
             destination_name = excluded.destination_name,
             destination_lat = excluded.destination_lat,
             destination_lon = excluded.destination_lon,
             aircraft_atc_type = excluded.aircraft_atc_type,
             aircraft_title = excluded.aircraft_title,
             aircraft_model = excluded.aircraft_model,
             aircraft_airline = excluded.aircraft_airline,
             aircraft_registration = excluded.aircraft_registration,
             distance_nm = excluded.distance_nm,
             flight_time_s = excluded.flight_time_s,
             max_altitude_ft = excluded.max_altitude_ft,
             landing_fpm = excluded.landing_fpm,
             max_ground_speed_kt = excluded.max_ground_speed_kt,
             max_true_airspeed_kt = excluded.max_true_airspeed_kt,
             departure_gate = excluded.departure_gate,
             arrival_gate = excluded.arrival_gate,
             passengers = excluded.passengers,
             cargo_kg = excluded.cargo_kg,
             fuel_used_kg = excluded.fuel_used_kg,
             paused_seconds = excluded.paused_seconds,
             flight_number = excluded.flight_number,
             callsign = excluded.callsign,
             airline_icao = excluded.airline_icao,
             status = excluded.status,
             score_total = excluded.score_total,
             score_max = excluded.score_max,
             score_grade = excluded.score_grade,
             metar_origin = COALESCE(excluded.metar_origin, flight_log.metar_origin),
             metar_dest = COALESCE(excluded.metar_dest, flight_log.metar_dest)"#,
    )
    .bind(id)
    .bind(started_at)
    .bind(json_str(row, "endedAt"))
    .bind(json_str(row, "originIcao"))
    .bind(json_str(row, "originName"))
    .bind(json_f64(row, "originLat").unwrap_or(0.0))
    .bind(json_f64(row, "originLon").unwrap_or(0.0))
    .bind(json_str(row, "destinationIcao"))
    .bind(json_str(row, "destinationName"))
    .bind(json_f64(row, "destinationLat"))
    .bind(json_f64(row, "destinationLon"))
    .bind(json_str(row, "aircraftAtcType"))
    .bind(json_str(row, "aircraftTitle"))
    .bind(json_str(row, "aircraftModel"))
    .bind(json_str(row, "aircraftAirline"))
    .bind(json_str(row, "aircraftRegistration"))
    .bind(json_f64(row, "distanceNm"))
    .bind(json_i64(row, "flightTimeS"))
    .bind(json_i64(row, "maxAltitudeFt"))
    .bind(json_i64(row, "landingFpm"))
    .bind(json_i64(row, "maxGroundSpeedKt"))
    .bind(json_i64(row, "maxTrueAirspeedKt"))
    .bind(json_str(row, "departureGate"))
    .bind(json_str(row, "arrivalGate"))
    .bind(json_i64(row, "passengers"))
    .bind(json_i64(row, "cargoKg"))
    .bind(json_i64(row, "fuelUsedKg"))
    .bind(json_i64(row, "pausedSeconds").unwrap_or(0))
    .bind(flight_number)
    .bind(json_str(row, "callsign"))
    .bind(airline_icao)
    .bind(json_str(row, "status").unwrap_or_else(|| "completed".to_string()))
    .bind(json_i64(row, "scoreTotal"))
    .bind(json_i64(row, "scoreMax"))
    .bind(json_str(row, "scoreGrade"))
    .bind(json_str(row, "metarOrigin"))
    .bind(json_str(row, "metarDest"))
}

async fn restore_snapshot(pool: &SqlitePool, snap: &Snapshot) -> anyhow::Result<RestoreReport> {
    // (v7 fix) ¿La DB estaba vacía antes del restore? Solo entonces el cleanup
    // de "phantom in-flight" es seguro: si hay datos locales, un UPDATE global
    // que cierra ended_at=NULL cerraría también un vuelo LOCAL en curso (que
    // aún no registró su primer punto). En ese caso NO barremos la tabla.
    let was_empty: bool = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM flight_log")
        .fetch_one(pool)
        .await
        .unwrap_or(0)
        == 0;
    // (24/10) Lo borrado a propósito no se restaura (ni sus tracks/fases).
    crate::ledger_sync::apply_remote_tombstones(pool, &snap.deleted).await?;
    let tombs = crate::ledger_sync::load_tombstones(pool).await?;
    let dead = crate::ledger_sync::active_keys(&tombs, crate::ledger_sync::KIND_FLIGHT);
    let dead_ids: std::collections::HashSet<i64> = snap
        .flight_log
        .iter()
        .filter(|r| crate::ledger_sync::flight_row_dead(r, &dead))
        .filter_map(|r| r.get("id").and_then(|v| v.as_i64()))
        .collect();
    // (v7.4.2) Mismo problema que `merge_remote_snapshot`: el snapshot puede
    // traer el mismo nº de vuelo el mismo día en varias sesiones y el índice
    // `uq_flight_log_va_dedup_completed` tumbaba el restore entero (lo vio el
    // auto-sync con la DB vacía). El nº lo conserva el vuelo MÁS LARGO de cada
    // grupo; si aun así choca (p.ej. con un vuelo local), entra sin nº.
    let mut va_winner: std::collections::HashMap<String, (i64, String, i64)> =
        std::collections::HashMap::new();
    let mut va_key_by_id: std::collections::HashMap<i64, String> =
        std::collections::HashMap::new();
    for row in &snap.flight_log {
        let Some(id) = row.get("id").and_then(|v| v.as_i64()) else { continue };
        let Some(key) = cloud_va_key(pool, row).await? else { continue };
        let time_s = json_i64(row, "flightTimeS").unwrap_or(0);
        let started = json_str(row, "startedAt").unwrap_or_default();
        let better = va_winner
            .get(&key)
            .map_or(true, |(t, st, _)| time_s > *t || (time_s == *t && started < *st));
        if better {
            va_winner.insert(key.clone(), (time_s, started, id));
        }
        va_key_by_id.insert(id, key);
    }
    let mut tx = pool.begin().await?;
    let mut flights = 0;
    let mut tracks = 0;
    let mut settings = 0;
    for row in &snap.flight_log {
        let id = match row.get("id").and_then(|v| v.as_i64()) {
            Some(v) => v,
            None => continue,
        };
        if dead_ids.contains(&id) {
            continue;
        }
        let started_at = json_str(row, "startedAt").unwrap_or_default();
        // (v3.6.0 Phase H) Extendido para incluir flight_number,
        // callsign, airline_icao, status y los score_* del scoring
        // engine. Los campos del scoring SE sincronizan al cloud porque
        // el usuario los puede consultar desde cualquier máquina sin
        // tener que recomputar (decisión Q15 del kickoff).
        let keep_number = va_key_by_id
            .get(&id)
            .map_or(true, |k| va_winner.get(k).is_some_and(|(_, _, w)| *w == id));
        let upsert = |with_number: bool| upsert_snapshot_flight(row, id, &started_at, with_number);
        if keep_number {
            match upsert(true).execute(&mut *tx).await {
                Ok(_) => {}
                Err(e) if e.to_string().contains("uq_flight_log_va_dedup_completed") => {
                    upsert(false).execute(&mut *tx).await?;
                }
                Err(e) => return Err(e.into()),
            }
        } else {
            upsert(false).execute(&mut *tx).await?;
        }
        complete_flight(&mut tx, row, id, true).await?;
        flights += 1;
    }
    for row in &snap.flight_log_track {
        let flight_id = json_i64(row, "flightId").unwrap_or(0);
        let ts = json_str(row, "ts").unwrap_or_default();
        if flight_id == 0 || ts.is_empty() || dead_ids.contains(&flight_id) {
            continue;
        }
        // (v2.2.0) FIX: la columna real en la tabla es `gs_kt`, no
        // `ground_speed_kt`. El JSON puede traer cualquiera.
        //
        // (v3.7.0 Phase O) Extendido para soportar los simvars VAS-aligned
        // que se sumaron a `flight_log_track`. Snapshots pre-v3.7 no
        // tienen estos campos → los binds caen a NULL.
        let b_bool = |key: &str| -> Option<i64> {
            json_i64(row, key).map(|v| if v != 0 { 1 } else { 0 })
        };
        // (v4.0.0 — P4) Extendido con métricas de motor por slot 1..4.
        // 24 columnas extra. Snapshots pre-v4 NO traen estas keys →
        // todos los binds caen a NULL.
        sqlx::query(
            r#"INSERT OR IGNORE INTO flight_log_track (
                flight_id, ts, lat, lon, alt_ft, gs_kt,
                ias_kt, vs_fpm, pitch_deg, bank_deg, g_force,
                flaps_pct, gear_down, spoilers_pct,
                light_nav, light_beacon, light_taxi, light_landing, light_strobe,
                parking_brake, transponder_code,
                eng_n1_1, eng_n1_2, eng_n1_3, eng_n1_4,
                eng_n2_1, eng_n2_2, eng_n2_3, eng_n2_4,
                eng_egt_1, eng_egt_2, eng_egt_3, eng_egt_4,
                eng_ff_pph_1, eng_ff_pph_2, eng_ff_pph_3, eng_ff_pph_4,
                eng_oil_temp_1, eng_oil_temp_2, eng_oil_temp_3, eng_oil_temp_4,
                eng_oil_press_1, eng_oil_press_2, eng_oil_press_3, eng_oil_press_4
            ) VALUES (?,?,?,?,?,?, ?,?,?,?,?, ?,?,?, ?,?,?,?,?, ?,?,
                      ?,?,?,?, ?,?,?,?, ?,?,?,?, ?,?,?,?, ?,?,?,?, ?,?,?,?)"#,
        )
        .bind(flight_id)
        .bind(&ts)
        .bind(json_f64(row, "lat").unwrap_or(0.0))
        .bind(json_f64(row, "lon").unwrap_or(0.0))
        .bind(json_i64(row, "altFt"))
        .bind(
            // (v2.2.0) Compat: snapshots viejas usan "groundSpeedKt",
            // las nuevas (dynamic row_to_json) usan "gsKt". Aceptar ambas.
            json_i64(row, "gsKt")
                .or_else(|| json_i64(row, "groundSpeedKt")),
        )
        .bind(json_i64(row, "iasKt"))
        .bind(json_i64(row, "vsFpm"))
        .bind(json_f64(row, "pitchDeg"))
        .bind(json_f64(row, "bankDeg"))
        .bind(json_f64(row, "gForce"))
        .bind(json_i64(row, "flapsPct"))
        .bind(b_bool("gearDown"))
        .bind(json_i64(row, "spoilersPct"))
        .bind(b_bool("lightNav"))
        .bind(b_bool("lightBeacon"))
        .bind(b_bool("lightTaxi"))
        .bind(b_bool("lightLanding"))
        .bind(b_bool("lightStrobe"))
        .bind(b_bool("parkingBrake"))
        .bind(json_i64(row, "transponderCode"))
        .bind(json_f64(row, "engN11"))
        .bind(json_f64(row, "engN12"))
        .bind(json_f64(row, "engN13"))
        .bind(json_f64(row, "engN14"))
        .bind(json_f64(row, "engN21"))
        .bind(json_f64(row, "engN22"))
        .bind(json_f64(row, "engN23"))
        .bind(json_f64(row, "engN24"))
        .bind(json_f64(row, "engEgt1"))
        .bind(json_f64(row, "engEgt2"))
        .bind(json_f64(row, "engEgt3"))
        .bind(json_f64(row, "engEgt4"))
        .bind(json_f64(row, "engFfPph1"))
        .bind(json_f64(row, "engFfPph2"))
        .bind(json_f64(row, "engFfPph3"))
        .bind(json_f64(row, "engFfPph4"))
        .bind(json_f64(row, "engOilTemp1"))
        .bind(json_f64(row, "engOilTemp2"))
        .bind(json_f64(row, "engOilTemp3"))
        .bind(json_f64(row, "engOilTemp4"))
        .bind(json_f64(row, "engOilPress1"))
        .bind(json_f64(row, "engOilPress2"))
        .bind(json_f64(row, "engOilPress3"))
        .bind(json_f64(row, "engOilPress4"))
        .execute(&mut *tx)
        .await?;
        tracks += 1;
    }
    for kv in &snap.settings {
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES (?,?,datetime('now')) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at"
        )
        .bind(&kv.key)
        .bind(&kv.value)
        .execute(&mut *tx)
        .await?;
        settings += 1;
    }

    // (v3.6.0 Phase H) Restore de score items y phase transitions.
    // INSERT OR REPLACE para que un re-restore con datos actualizados
    // (re-scoring desde otra máquina) pise el local. La FK con CASCADE
    // garantiza que estos quedaron limpios si el vuelo padre se borró.
    for row in &snap.flight_log_score_item {
        let Some(flight_id) = json_i64(row, "flightId").filter(|id| !dead_ids.contains(id)) else { continue };
        let Some(phase) = json_str(row, "phase") else { continue };
        let Some(rule_id) = json_str(row, "ruleId") else { continue };
        sqlx::query(
            r#"INSERT INTO flight_log_score_item
                (flight_id, phase, rule_id, label, points_earned, points_max,
                 passed, severity, evidence, ts)
               VALUES (?,?,?,?,?,?,?,?,?,?)
               ON CONFLICT(flight_id, phase, rule_id) DO UPDATE SET
                 label         = excluded.label,
                 points_earned = excluded.points_earned,
                 points_max    = excluded.points_max,
                 passed        = excluded.passed,
                 severity      = excluded.severity,
                 evidence      = excluded.evidence,
                 ts            = excluded.ts"#,
        )
        .bind(flight_id)
        .bind(&phase)
        .bind(&rule_id)
        .bind(json_str(row, "label").unwrap_or_default())
        .bind(json_i64(row, "pointsEarned").unwrap_or(0))
        .bind(json_i64(row, "pointsMax").unwrap_or(0))
        .bind(json_i64(row, "passed").unwrap_or(0))
        .bind(json_str(row, "severity"))
        .bind(json_str(row, "evidence"))
        .bind(json_str(row, "ts"))
        .execute(&mut *tx)
        .await?;
    }

    for row in &snap.flight_log_phase {
        let Some(flight_id) = json_i64(row, "flightId").filter(|id| !dead_ids.contains(id)) else { continue };
        let Some(phase) = json_str(row, "phase") else { continue };
        let Some(entered_at) = json_str(row, "enteredAt") else { continue };
        sqlx::query(
            r#"INSERT INTO flight_log_phase
                (flight_id, phase, entered_at, exited_at)
               VALUES (?,?,?,?)
               ON CONFLICT(flight_id, phase, entered_at) DO UPDATE SET
                 exited_at = excluded.exited_at"#,
        )
        .bind(flight_id)
        .bind(&phase)
        .bind(&entered_at)
        .bind(json_str(row, "exitedAt"))
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    // (v3.5.0 F3) Cierra cualquier vuelo con ended_at = NULL que NO
    // tenga datos de posición en vivo. Estos son "phantom in-flight"
    // imported desde otra máquina — si nunca tuvieron last_position
    // local, no son vuelos que estemos volando AHORA. Sin este cleanup,
    // el FlightBook los mostraba como "FLYING NOW".
    if was_empty {
        let closed = sqlx::query(
            r#"UPDATE flight_log
               SET ended_at = started_at
               WHERE ended_at IS NULL
                 AND last_position_at IS NULL"#,
        )
        .execute(pool)
        .await?;
        if closed.rows_affected() > 0 {
            tracing::info!(
                target: "cloud",
                "restore cleanup: auto-cerrados {} vuelos phantom (sin last_position)",
                closed.rows_affected()
            );
        }
    }

    // (24/10) El libro (Tech Log, cobros, GSX…) y lo derivado.
    crate::ledger_sync::apply(pool, &snap.ledger, &tombs).await?;
    settle_after_sync(pool).await;

    Ok(RestoreReport {
        flights,
        tracks,
        settings,
    })
}

/// (24/10) Columnas del vuelo que los INSERT de la nube no llevaban (se
/// añadieron después): sin ellas otra PC calculaba otro desgaste y otro
/// dinero (frenos, neumáticos, rebotes) y perdía la ruta y el OFP.
/// `with_source`: el restore por id tampoco llevaba origen ni id externo.
async fn complete_flight(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    row: &serde_json::Value,
    id: i64,
    with_source: bool,
) -> anyhow::Result<()> {
    const EXTRA: &[(&str, &str)] = &[
        ("simbriefOfpId", "simbrief_ofp_id"),
        ("bounced", "bounced"),
        ("maxBrakeTempC", "max_brake_temp_c"),
        ("routeFixes", "route_fixes"),
        ("partialAck", "partial_ack"),
        ("maxTireWearPct", "max_tire_wear_pct"),
        ("source", "source"),
        ("externalId", "external_id"),
    ];
    for (key, col) in EXTRA {
        if !with_source && matches!(*key, "source" | "externalId") {
            continue;
        }
        let q = format!("UPDATE flight_log SET {col} = ?1 WHERE id = ?2");
        let q = match row.get(*key) {
            Some(serde_json::Value::String(v)) => sqlx::query(&q).bind(v.clone()),
            Some(serde_json::Value::Number(n)) => match n.as_i64() {
                Some(i) => sqlx::query(&q).bind(i),
                None => sqlx::query(&q).bind(n.as_f64()),
            },
            _ => continue,
        };
        q.bind(id).execute(&mut **tx).await?;
    }
    Ok(())
}

fn json_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(String::from)
}
fn json_f64(v: &serde_json::Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| x.as_f64())
}
fn json_i64(v: &serde_json::Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64())
}

#[derive(Debug, Default)]
struct RestoreReport {
    flights: usize,
    tracks: usize,
    settings: usize,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub uploaded_flights: usize,
    pub uploaded_tracks: usize,
    pub uploaded_settings: usize,
    pub downloaded_flights: usize,
    pub downloaded_tracks: usize,
    pub downloaded_settings: usize,
}

/// (v2.0.2) Diagnóstico paso a paso del flow OAuth + Drive API.
/// Ejecuta cada call en orden y devuelve `[ok|error, descripción]`
/// por cada paso para que el frontend muestre dónde se rompe la
/// cadena. Útil cuando algo falla y el usuario no sabe en qué paso
/// concreto.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CloudTestStep {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CloudTestReport {
    pub overall_ok: bool,
    pub steps: Vec<CloudTestStep>,
    pub hint: Option<String>,
}

pub async fn test_connection(
    pool: &SqlitePool,
    http: &reqwest::Client,
) -> CloudTestReport {
    let mut steps = Vec::new();
    let mut hint: Option<String> = None;

    // Paso 1 — Credenciales locales.
    //
    // (v3.6.2 fix I12) Usar `resolve_credentials` para que mire PRIMERO
    // los valores hardcoded del build (`option_env!` desde build.rs +
    // secrets.local.toml) y SOLO si faltan caiga al fallback de DB.
    //
    // El bug previo: este paso leía SOLO la DB, lo que daba "Falta
    // Client ID" aunque el binario tuviera los secrets embedidos y
    // todos los demás flujos (connect, upload, download) funcionaran.
    // El mensaje también mencionaba un botón "Configurar credenciales"
    // que YA NO existe (eliminado en task #85 — Cloud UI cleanup) —
    // pista falsa para el usuario.
    let creds = resolve_credentials(pool).await;
    let (client_id, client_secret) = match creds {
        Ok(pair) => pair,
        Err(e) => {
            steps.push(CloudTestStep {
                name: crate::tr!("cloudSync.stepLocalCreds"),
                ok: false,
                detail: crate::tr!("cloudSync.credsQueryError", e = e),
            });
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint: None,
            };
        }
    };
    let cid_ok = client_id.as_deref().filter(|s| !s.is_empty()).is_some();
    let secret_ok = client_secret.as_deref().filter(|s| !s.is_empty()).is_some();
    if !cid_ok || !secret_ok {
        steps.push(CloudTestStep {
            name: crate::tr!("cloudSync.stepLocalCreds"),
            ok: false,
            detail: crate::tr!("cloudSync.noClientIdSecretEmbedded"),
        });
        return CloudTestReport {
            overall_ok: false,
            steps,
            hint: Some(crate::tr!("cloudSync.hintCredsEmbedded")),
        };
    }
    let source = if HARDCODED_CLIENT_ID
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        crate::tr!("cloudSync.credsSourceEmbedded")
    } else {
        crate::tr!("cloudSync.credsSourceDb")
    };
    steps.push(CloudTestStep {
        name: crate::tr!("cloudSync.stepLocalCreds"),
        ok: true,
        detail: crate::tr!(
            "cloudSync.credsOkDetail",
            chars = client_id.as_ref().map(|s| s.len()).unwrap_or(0),
            source = source
        ),
    });

    // Paso 2 — Refresh token
    let refresh_token = get_setting(pool, KEY_REFRESH_TOKEN).await.ok().flatten();
    if refresh_token.is_none() {
        steps.push(CloudTestStep {
            name: crate::tr!("cloudSync.stepRefreshToken"),
            ok: false,
            detail: crate::tr!("cloudSync.oauthNotCompletedDetail"),
        });
        return CloudTestReport {
            overall_ok: false,
            steps,
            hint: Some(crate::tr!("cloudSync.hintCompleteOauth")),
        };
    }
    steps.push(CloudTestStep {
        name: crate::tr!("cloudSync.stepRefreshToken"),
        ok: true,
        detail: crate::tr!("cloudSync.refreshTokenSaved"),
    });

    // Paso 3 — Access token (refresh)
    let access_token = match fresh_access_token(pool, http).await {
        Ok(t) => {
            steps.push(CloudTestStep {
                name: crate::tr!("cloudSync.stepRefreshAccessToken"),
                ok: true,
                detail: crate::tr!("cloudSync.accessTokenObtained", chars = t.len()),
            });
            t
        }
        Err(e) => {
            steps.push(CloudTestStep {
                name: crate::tr!("cloudSync.stepRefreshAccessToken"),
                ok: false,
                detail: format!("{}", e),
            });
            hint = Some(crate::tr!("cloudSync.hintRefreshRejected"));
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint,
            };
        }
    };

    // Paso 4 — userinfo (no requiere Drive API)
    match http
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .bearer_auth(&access_token)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let email = resp
                .json::<UserInfo>()
                .await
                .map(|u| u.email)
                .unwrap_or_else(|_| crate::tr!("cloudSync.noEmailPlaceholder"));
            steps.push(CloudTestStep {
                name: crate::tr!("cloudSync.stepUserinfo"),
                ok: true,
                detail: crate::tr!("cloudSync.connectedAsDetail", email = email),
            });
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            steps.push(CloudTestStep {
                name: crate::tr!("cloudSync.stepUserinfo"),
                ok: false,
                detail: format!("{}: {}", status, body),
            });
            hint = Some(crate::tr!("cloudSync.hintUserinfoFailed"));
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint,
            };
        }
        Err(e) => {
            steps.push(CloudTestStep {
                name: crate::tr!("cloudSync.stepUserinfo"),
                ok: false,
                detail: crate::tr!("cloudSync.networkErrorDetail", e = e),
            });
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint: Some(crate::tr!("cloudSync.hintNoInternet")),
            };
        }
    }

    // Paso 5 — drive/about (requiere Drive API + scope drive.appdata)
    match http
        .get("https://www.googleapis.com/drive/v3/about?fields=user(emailAddress),storageQuota(usage,limit)")
        .bearer_auth(&access_token)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            steps.push(CloudTestStep {
                name: crate::tr!("cloudSync.stepDriveReachable"),
                ok: true,
                detail: crate::tr!("cloudSync.driveAboutOk"),
            });
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            steps.push(CloudTestStep {
                name: crate::tr!("cloudSync.stepDriveReachable"),
                ok: false,
                detail: format!("{}: {}", status, body),
            });
            if status.as_u16() == 403 {
                hint = Some(crate::tr!("cloudSync.hintDrive403"));
            }
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint,
            };
        }
        Err(e) => {
            steps.push(CloudTestStep {
                name: crate::tr!("cloudSync.stepDriveReachable"),
                ok: false,
                detail: crate::tr!("cloudSync.networkErrorDetail", e = e),
            });
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint: Some(crate::tr!("cloudSync.networkErrorHint")),
            };
        }
    }

    // Paso 6 — listar appDataFolder (lo que sync_now hace primero)
    let q = format!("name = '{}' and 'appDataFolder' in parents", crate::test_env::sync_upload_file_name());
    let url = format!(
        "https://www.googleapis.com/drive/v3/files?spaces=appDataFolder&q={}&fields=files(id)",
        urlencoding::encode(&q)
    );
    match http.get(&url).bearer_auth(&access_token).send().await {
        Ok(resp) if resp.status().is_success() => {
            let list: DriveList = resp.json().await.unwrap_or(DriveList { files: vec![] });
            let detail = if list.files.is_empty() {
                crate::tr!("cloudSync.appDataFolderEmpty")
            } else {
                crate::tr!("cloudSync.appDataFolderFound", n = list.files.len())
            };
            steps.push(CloudTestStep {
                name: crate::tr!("cloudSync.stepListAppDataFolder"),
                ok: true,
                detail,
            });
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            steps.push(CloudTestStep {
                name: crate::tr!("cloudSync.stepListAppDataFolder"),
                ok: false,
                detail: format!("{}: {}", status, body),
            });
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint: Some(crate::tr!("cloudSync.hintScopeNotGranted")),
            };
        }
        Err(e) => {
            steps.push(CloudTestStep {
                name: crate::tr!("cloudSync.stepListAppDataFolder"),
                ok: false,
                detail: crate::tr!("cloudSync.networkErrorDetail", e = e),
            });
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint: Some(crate::tr!("cloudSync.networkErrorHint")),
            };
        }
    }

    CloudTestReport {
        overall_ok: true,
        steps,
        hint: Some(crate::tr!("cloudSync.hintAllOk")),
    }
}

/// (v7 fix) Lock GLOBAL de las operaciones read-modify-write del snapshot en
/// la nube. `sync_now` (auto-sync diario), `upload_all` (auto-subida al
/// importar VAS/scoring) y `download_missing`/`load_from_folder` se disparan
/// como tareas tokio independientes; sin serializar, dos subidas solapadas
/// bajan el mismo snapshot base, mergean por separado y la última pisa a la
/// primera (lost update). Este mutex las serializa. NO es reentrante: cuidar
/// de no anidar (sync_now llama a download_missing/upload_all, que lo toman
/// ellos mismos — sync_now no lo toma en ese camino).
static CLOUD_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn sync_now(
    pool: &SqlitePool,
    http: &reqwest::Client,
) -> anyhow::Result<SyncReport> {
    let mut report = SyncReport::default();

    // (v7 fix, pérdida de datos) NO usar restore_snapshot (upsert POR id) para
    // mergear: el id de flight_log es un autoincrement LOCAL, así que el id=5
    // de una máquina y el id=5 de otra son vuelos DISTINTOS → el upsert pisaba
    // vuelos locales con los del cloud que casualmente comparten id.
    //   · DB vacía (reinstalación) → restore completo por id es seguro.
    //   · DB con datos → merge por CLAVES ESTABLES (download_missing), que
    //     inserta con id fresco y remapea tracks. Es el mismo camino probado
    //     del botón "Bajar".
    let local_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM flight_log")
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    if local_count == 0 {
        let _guard = CLOUD_LOCK.lock().await;
        let access_token = fresh_access_token(pool, http).await?;
        if let Some(text) = download_sync_file(http, &access_token, None, crate::test_env::sync_download_file_name()).await? {
            match serde_json::from_str::<Snapshot>(&text) {
                Ok(remote) => {
                    let r = restore_snapshot(pool, &remote).await?;
                    report.downloaded_flights = r.flights;
                    report.downloaded_tracks = r.tracks;
                    report.downloaded_settings = r.settings;
                }
                Err(e) => {
                    tracing::warn!(
                        target: "cloud",
                        "snapshot remota corrupta — la sobreescribimos con la local: {e:#}"
                    );
                }
            }
        }
    } else {
        let dl = download_missing(pool, http, None).await?;
        report.downloaded_flights = dl.new_flights;
        report.downloaded_tracks = dl.new_tracks;
        report.downloaded_settings = dl.new_settings;
    }

    // Subida: merge incremental local → nube (idempotente, con su propio lock).
    let up = upload_all(pool, http, None).await?;
    report.uploaded_flights = up.uploaded_flights;
    report.uploaded_tracks = up.uploaded_tracks;
    report.uploaded_settings = up.uploaded_settings;

    set_setting(
        pool,
        KEY_LAST_SYNC_AT,
        &chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    )
    .await?;
    Ok(report)
}

// =============================================================================
// (v3.5.0 F3) Split upload / download — el usuario reportó que el sync
// "todo-en-uno" confunde: no se sabe si va a subir, bajar, o mergear.
// Botones separados con semántica clara.
// =============================================================================

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UploadReport {
    pub uploaded_flights: usize,
    pub uploaded_tracks: usize,
    pub uploaded_settings: usize,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DownloadReport {
    pub cloud_flights: usize,
    pub new_flights: usize,
    pub skipped_existing: usize,
    pub new_tracks: usize,
    pub new_settings: usize,
    /// (24/10) Vuelos borrados aquí porque se borraron en otra PC.
    pub deleted_flights: usize,
    /// (24/10) Cambios del libro traídos (aviones, cobros, líneas GSX…).
    pub ledger_changes: usize,
}

/// (24/10) Tras bajar vuelos, borrados o el libro: lo derivado se rehace
/// (Tech Log de vuelos que ya no están, P&L huérfanos, economía).
async fn settle_after_sync(pool: &SqlitePool) {
    crate::flight_log::cascade_after_delete(pool).await;
}

/// (24/10) Lo que `upload_all` sube: la nube (`merged`) + lo de esta PC, sin
/// lo borrado. Separado de la red para poder probarlo.
struct UploadMerge {
    snapshot: Snapshot,
    report: UploadReport,
    /// ¿Hay algo que la nube aún no tenga?
    changed: bool,
    removed_flights: usize,
    ledger_changed: bool,
}

async fn merge_for_upload(pool: &SqlitePool, mut merged: Snapshot) -> anyhow::Result<UploadMerge> {
    use std::collections::{HashMap, HashSet};
    // 2. (24/10) Borrados: los de otras PCs se aplican aquí antes de subir
    //    (si no, esta PC volvería a subir lo borrado), y los vuelos que se
    //    reimportaron aquí después de borrarlos dejan de estar borrados.
    if crate::ledger_sync::apply_remote_tombstones(pool, &merged.deleted).await? > 0 {
        settle_after_sync(pool).await;
    }
    crate::ledger_sync::revive_readded(pool).await?;

    // Snapshot local (todo lo que hay en este PC).
    let local = build_snapshot(pool).await?;

    // Lo borrado sale de la nube (vuelos con sus tracks, fases y puntos).
    let tombs = crate::ledger_sync::union_tombstones(&merged.deleted, &local.deleted);
    let dead = crate::ledger_sync::active_keys(&tombs, crate::ledger_sync::KIND_FLIGHT);
    let dead_ids: HashSet<i64> = merged
        .flight_log
        .iter()
        .filter(|r| crate::ledger_sync::flight_row_dead(r, &dead))
        .filter_map(|r| r.get("id").and_then(|v| v.as_i64()))
        .collect();
    let removed_flights = merged.flight_log.len();
    merged.flight_log.retain(|r| !crate::ledger_sync::flight_row_dead(r, &dead));
    let removed_flights = removed_flights - merged.flight_log.len();
    if !dead_ids.is_empty() {
        let alive = |r: &serde_json::Value| !r.get("flightId").and_then(|v| v.as_i64()).is_some_and(|id| dead_ids.contains(&id));
        merged.flight_log_track.retain(alive);
        merged.flight_log_phase.retain(alive);
        merged.flight_log_score_item.retain(alive);
    }

    // Libro: unión de la nube y esta PC, sin lo borrado.
    let ledger = crate::ledger_sync::merge(&merged.ledger, &local.ledger, &tombs);
    let ledger_changed = ledger != merged.ledger;
    let deleted_changed = tombs != crate::ledger_sync::union_tombstones(&merged.deleted, &[]);
    merged.ledger = ledger;
    merged.deleted = tombs;

    // 3. Índice de claves estables de los vuelos que YA están en la nube
    //    + el id máximo usado (para asignar ids nuevos sin colisión).
    let mut cloud_keys: HashSet<String> = HashSet::new();
    let mut max_id: i64 = 0;
    for row in &merged.flight_log {
        let (ik, sk) = snapshot_flight_keys(row);
        if let Some(k) = ik {
            cloud_keys.insert(k);
        }
        cloud_keys.insert(sk);
        if let Some(id) = row.get("id").and_then(|v| v.as_i64()) {
            max_id = max_id.max(id);
        }
    }

    // 4. Añadimos SOLO los vuelos locales que no están en la nube. Cada
    //    vuelo nuevo recibe un id fresco (sin colisión) y sus tracks /
    //    fases / score-items se remapean a ese id. Los vuelos que ya están
    //    arriba NO se re-suben ni se pisan (se preserva lo de otras máquinas).
    let mut next_id = max_id;
    let mut new_flights = 0usize;
    let mut new_tracks = 0usize;
    for frow in &local.flight_log {
        let (ik, sk) = snapshot_flight_keys(frow);
        let in_cloud = ik.as_ref().map(|k| cloud_keys.contains(k)).unwrap_or(false)
            || cloud_keys.contains(&sk);
        if in_cloud {
            continue;
        }
        let Some(old_id) = frow.get("id").and_then(|v| v.as_i64()) else {
            continue;
        };
        next_id += 1;
        let new_id = next_id;
        let mut f = frow.clone();
        if let Some(o) = f.as_object_mut() {
            o.insert("id".into(), serde_json::json!(new_id));
        }
        merged.flight_log.push(f);
        new_flights += 1;

        let remap = |rows: &[serde_json::Value], dst: &mut Vec<serde_json::Value>| -> usize {
            let mut n = 0;
            for r in rows {
                if r.get("flightId").and_then(|v| v.as_i64()) == Some(old_id) {
                    let mut c = r.clone();
                    if let Some(o) = c.as_object_mut() {
                        o.insert("flightId".into(), serde_json::json!(new_id));
                    }
                    dst.push(c);
                    n += 1;
                }
            }
            n
        };
        new_tracks += remap(&local.flight_log_track, &mut merged.flight_log_track);
        remap(&local.flight_log_phase, &mut merged.flight_log_phase);
        remap(&local.flight_log_score_item, &mut merged.flight_log_score_item);
    }

    // 5. Settings (pref_*): unión con la local como autoridad (son prefs de
    //    esta máquina). Pequeñas; no afectan al peso.
    let mut smap: HashMap<String, String> = merged
        .settings
        .iter()
        .map(|kv| (kv.key.clone(), kv.value.clone()))
        .collect();
    let settings_before = smap.len();
    for kv in &local.settings {
        smap.insert(kv.key.clone(), kv.value.clone());
    }
    let settings_changed = smap.len() != settings_before
        || local.settings.iter().any(|kv| {
            merged
                .settings
                .iter()
                .find(|m| m.key == kv.key)
                .map(|m| m.value != kv.value)
                .unwrap_or(true)
        });
    merged.settings = smap
        .into_iter()
        .map(|(key, value)| SettingKV { key, value })
        .collect();
    merged.version = SNAPSHOT_VERSION;
    merged.exported_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let report = UploadReport {
        uploaded_flights: new_flights,
        uploaded_tracks: new_tracks,
        uploaded_settings: merged.settings.len(),
    };

    let changed = new_flights > 0 || settings_changed || removed_flights > 0 || ledger_changed || deleted_changed;
    Ok(UploadMerge { snapshot: merged, report, changed, removed_flights, ledger_changed })
}

/// **Subir** TODO el contenido local a la nube (sobrescribe el snapshot
/// remoto). Incluye vuelos sim-flown + imports (VAS-ACARS) + tracks +
/// settings con prefijo `pref_`. Operación idempotente: re-subir no
/// duplica nada porque el cloud guarda un único snapshot.
pub async fn upload_all(
    pool: &SqlitePool,
    http: &reqwest::Client,
    app: Option<&AppHandle>,
) -> anyhow::Result<UploadReport> {
    // (v7 fix, race) Serializa con las demás operaciones RMW del snapshot cloud.
    let _cloud_guard = CLOUD_LOCK.lock().await;
    tracing::info!(target: "cloud", "upload_all: merge incremental local → nube…");
    // (v6.2.16) Marca el subsistema "cloud" como ocupado mientras dura la
    // subida — el panel de Diagnóstico lo muestra en vivo. El guard garantiza
    // el reset aunque salgamos por error (`?`).
    crate::diagnostics::note_cloud_busy(true);
    struct BusyGuard;
    impl Drop for BusyGuard {
        fn drop(&mut self) {
            crate::diagnostics::note_cloud_busy(false);
        }
    }
    let _busy = BusyGuard;
    let access_token = fresh_access_token(pool, http).await?;

    // 1. Snapshot remoto actual (si existe) — base del merge. Si está
    //    corrupto, partimos de vacío (recuperación: se reescribe entero).
    let remote_file_id = find_sync_file(http, &access_token, crate::test_env::sync_upload_file_name()).await?;
    // La bajada interna del snapshot remoto (para mergear) NO emite
    // progreso — así la barra de "Subir" solo muestra la fase de subida.
    let cloud: Snapshot = match download_sync_file(http, &access_token, None, crate::test_env::sync_upload_file_name()).await? {
        Some(t) => serde_json::from_str(&t).unwrap_or_default(),
        None => Snapshot::default(),
    };

    let UploadMerge { snapshot: merged, report, changed, removed_flights, ledger_changed } = merge_for_upload(pool, cloud).await?;

    // 6. Si no hay nada nuevo que añadir y los settings no cambiaron, y ya
    //    existe un snapshot remoto, no re-subimos el archivo entero — era
    //    justo lo que el usuario no quería ("que no suba todo otra vez").
    if !changed && remote_file_id.is_some() {
        tracing::info!(
            target: "cloud",
            "upload_all: nada nuevo que subir — la nube ya tiene todos los vuelos locales"
        );
        // Nada que subir → emitimos un done con tamaño 0 para que la barra
        // cierre limpio (el frontend igual tiene su red de seguridad).
        emit_transfer_progress(app, "upload", 0, 0, true);
        set_setting(
            pool,
            KEY_LAST_SYNC_AT,
            &chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        )
        .await?;
        return Ok(report);
    }

    let payload = serde_json::to_string(&merged)?;
    tracing::info!(
        target: "cloud",
        "upload_all: merge listo — +{} vuelos nuevos (+{} tracks), −{} borrados · libro {} · nube ahora {} vuelos · JSON {:.1} MB (gz)",
        report.uploaded_flights,
        report.uploaded_tracks,
        removed_flights,
        if ledger_changed { "actualizado" } else { "sin cambios" },
        merged.flight_log.len(),
        payload.len() as f64 / 1_048_576.0,
    );
    // update/create_sync_file emite el evento final con el tamaño subido.
    if let Some(file_id) = remote_file_id {
        update_sync_file(http, &access_token, &file_id, &payload, app).await?;
    } else {
        create_sync_file(http, &access_token, &payload, app).await?;
    }
    set_setting(
        pool,
        KEY_LAST_SYNC_AT,
        &chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    )
    .await?;
    tracing::info!(
        target: "cloud",
        "upload OK · +{} flights nuevos, +{} tracks",
        report.uploaded_flights, report.uploaded_tracks,
    );
    Ok(report)
}

/// (v4.14.1) Claves estables de un row de flight_log (snapshot JSON) para
/// dedup cross-machine — mismas que usa `download_missing`:
///   · import_key = `{source}|{externalId}` (sólo imports VAS-ACARS).
///   · sim_key    = `{startedAt}|{originLat:.2}|{originLon:.2}`.
fn snapshot_flight_keys(row: &serde_json::Value) -> (Option<String>, String) {
    let source = json_str(row, "source").unwrap_or_else(|| "simconnect".into());
    let external_id = json_str(row, "externalId");
    let started_at = json_str(row, "startedAt").unwrap_or_default();
    let origin_lat = json_f64(row, "originLat").unwrap_or(0.0);
    let origin_lon = json_f64(row, "originLon").unwrap_or(0.0);
    let import_key = external_id.as_deref().map(|eid| format!("{source}|{eid}"));
    let sim_key = format!("{started_at}|{origin_lat:.2}|{origin_lon:.2}");
    (import_key, sim_key)
}

/// **Bajar** los vuelos faltantes de la nube. NO sobrescribe los vuelos
/// que ya existen localmente — los detecta por:
///   1. `(source, external_id)` cuando el vuelo es un import (VAS-ACARS).
///   2. `(started_at, origin_icao || origin_lat::round_2)` cuando es
///      sim-flown — sin external_id, la combinación timestamp+origen
///      es estable.
/// Los vuelos nuevos se insertan con ID auto-asignado (NO se usa el ID
/// del cloud para evitar conflictos cross-machine). Los tracks
/// asociados a vuelos nuevos también se importan, mapeando el ID viejo
/// del cloud al nuevo local.
///
/// Auto-cierre: si un vuelo descargado tiene `ended_at = NULL` (estaba
/// en curso en otra máquina al momento del upload), lo cerramos con
/// `ended_at = started_at` para que no aparezca como "FLYING NOW"
/// fantasma en esta máquina.
pub async fn download_missing(
    pool: &SqlitePool,
    http: &reqwest::Client,
    app: Option<&AppHandle>,
) -> anyhow::Result<DownloadReport> {
    // (v7 fix, race) Serializa con las demás operaciones RMW del snapshot cloud.
    let _cloud_guard = CLOUD_LOCK.lock().await;
    let access_token = fresh_access_token(pool, http).await?;

    // download_sync_file emite el evento final con el tamaño bajado.
    let Some(text) = download_sync_file(http, &access_token, app, crate::test_env::sync_download_file_name()).await? else {
        tracing::info!(target: "cloud", "download: no remote snapshot exists yet");
        return Ok(DownloadReport::default());
    };
    let remote: Snapshot = match serde_json::from_str(&text) {
        Ok(s) => s,
        Err(e) => {
            return Err(anyhow!("{}", crate::tr!("cloudSync.remoteSnapshotCorrupt", e = format!("{e:#}"))));
        }
    };
    let report = merge_remote_snapshot(pool, &remote).await?;

    set_setting(
        pool,
        KEY_LAST_SYNC_AT,
        &chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    )
    .await?;

    tracing::info!(
        target: "cloud",
        "download OK · cloud={} new={} skipped={} tracks={} settings={}",
        report.cloud_flights,
        report.new_flights,
        report.skipped_existing,
        report.new_tracks,
        report.new_settings,
    );
    Ok(report)
}

/// (v7.4.2) Fusiona un snapshot ya descargado en la DB local (el corazón de
/// `download_missing`, separado de la red para poder testearlo con datos
/// reales). Todo en UNA transacción: o entra el snapshot entero o nada.
async fn merge_remote_snapshot(
    pool: &SqlitePool,
    remote: &Snapshot,
) -> anyhow::Result<DownloadReport> {
    // (24/10) Primero los borrados de otras PCs: esos vuelos se van de aquí
    // (con su Tech Log y su dinero) y no se vuelven a bajar.
    let mut report = DownloadReport {
        cloud_flights: remote.flight_log.len(),
        deleted_flights: crate::ledger_sync::apply_remote_tombstones(pool, &remote.deleted).await?,
        ..Default::default()
    };
    let tombs = crate::ledger_sync::load_tombstones(pool).await?;
    let dead = crate::ledger_sync::active_keys(&tombs, crate::ledger_sync::KIND_FLIGHT);

    // Construye índice de claves estables existentes en local para
    // hacer dedup eficiente. Un vuelo coincide si:
    //  - (source, external_id) matchea, o
    //  - (started_at, origin_lat round 2) matchea (sim-flown sin
    //    external_id)
    let mut local_keys = build_local_dedup_keys(pool).await?;

    // (v7.4.2) Dedup del índice `uq_flight_log_va_dedup_completed` (UN solo
    // vuelo completed por aerolínea + nº de vuelo + día). Las claves de arriba
    // no lo cubren: el snapshot (JSON, sin constraints) puede traer el mismo
    // nº el mismo día en sesiones distintas — p.ej. un vuelo real + sesiones
    // abortadas — y el 2º INSERT tumbaba TODA la bajada con
    // "UNIQUE constraint failed: index 'uq_flight_log_va_dedup_completed'".
    // Misma política que el auto-link de SimBrief (`link_ofp_minimal`): se
    // bajan TODOS los vuelos y el nº lo conserva uno solo — el que ya exista
    // en local o, si no, el MÁS LARGO del snapshot; el resto entra sin
    // flight_number/airline_icao (el callsign se conserva).
    let mut local_va = build_local_va_keys(pool).await?;
    let mut va_key_of: Vec<Option<String>> = Vec::with_capacity(remote.flight_log.len());
    // clave VA → (flight_time_s, started_at, índice en el snapshot) del ganador.
    let mut va_winner: std::collections::HashMap<String, (i64, String, usize)> =
        std::collections::HashMap::new();
    for (idx, row) in remote.flight_log.iter().enumerate() {
        let key = cloud_va_key(pool, row).await?;
        if let Some(k) = key.as_ref().filter(|k| !local_va.contains(*k)) {
            let time_s = json_i64(row, "flightTimeS").unwrap_or(0);
            let started = json_str(row, "startedAt").unwrap_or_default();
            let better = va_winner
                .get(k)
                .map_or(true, |(t, st, _)| time_s > *t || (time_s == *t && started < *st));
            if better {
                va_winner.insert(k.clone(), (time_s, started, idx));
            }
        }
        va_key_of.push(key);
    }
    let mut va_stripped = 0usize;

    // Mapa de cloud_flight_id → new_local_id (para tracks).
    let mut cloud_to_local_id: std::collections::HashMap<i64, i64> =
        std::collections::HashMap::new();

    let mut tx = pool.begin().await?;

    for (idx, row) in remote.flight_log.iter().enumerate() {
        let cloud_id = json_i64(row, "id");
        let source = json_str(row, "source").unwrap_or_else(|| "simconnect".into());
        let external_id = json_str(row, "externalId");
        let started_at = json_str(row, "startedAt").unwrap_or_default();
        let origin_lat = json_f64(row, "originLat").unwrap_or(0.0);
        let origin_lon = json_f64(row, "originLon").unwrap_or(0.0);

        // Dedup check
        let import_key = external_id
            .as_deref()
            .map(|eid| format!("{}|{}", source, eid));
        let sim_key = format!("{}|{:.2}|{:.2}", started_at, origin_lat, origin_lon);
        let exists = import_key
            .as_ref()
            .map(|k| local_keys.contains(k))
            .unwrap_or(false)
            || local_keys.contains(&sim_key);

        if exists {
            report.skipped_existing += 1;
            continue;
        }
        if crate::ledger_sync::flight_row_dead(row, &dead) {
            continue;
        }

        // Auto-cierre del ended_at si es NULL (vuelo "en curso" en
        // otra máquina) — evita el bug de FLYING NOW phantom.
        let ended_at_cloud = json_str(row, "endedAt");
        let ended_at = match ended_at_cloud {
            Some(e) if !e.is_empty() => Some(e),
            _ => {
                // Si el cloud no lo cerró, lo cerramos artificialmente
                // con ended_at = started_at. Mejor que phantom.
                Some(started_at.clone())
            }
        };

        // (v7.4.2) ¿Este vuelo conserva el nº de vuelo? Sólo si no choca con
        // el índice VA (ver el pre-pase de arriba).
        let va_key = va_key_of[idx].clone();
        let keep_number = match &va_key {
            None => true,
            Some(k) => {
                !local_va.contains(k) && va_winner.get(k).is_some_and(|(_, _, w)| *w == idx)
            }
        };
        let insert = |with_number: bool| {
            insert_cloud_flight(
                row,
                &started_at,
                ended_at.as_deref(),
                origin_lat,
                origin_lon,
                &source,
                external_id.as_deref(),
                with_number,
            )
        };
        let result = if keep_number {
            match insert(true).execute(&mut *tx).await {
                Ok(r) => r,
                // Red de seguridad: si aun así choca (p.ej. un formato de
                // fecha raro), entra sin nº en vez de abortar toda la bajada.
                // SQLite sólo aborta la SENTENCIA; la transacción sigue viva.
                Err(e) if e.to_string().contains("uq_flight_log_va_dedup_completed") => {
                    va_stripped += 1;
                    insert(false).execute(&mut *tx).await?
                }
                Err(e) => return Err(e.into()),
            }
        } else {
            va_stripped += 1;
            insert(false).execute(&mut *tx).await?
        };
        if keep_number {
            if let Some(k) = va_key {
                local_va.insert(k);
            }
        }
        // (v7.4.2) Lo recién insertado cuenta para el dedup del RESTO del
        // snapshot: antes un vuelo repetido dentro del propio snapshot entraba
        // dos veces.
        if let Some(k) = import_key {
            local_keys.insert(k);
        }
        local_keys.insert(sim_key);

        let new_id = result.last_insert_rowid();
        complete_flight(&mut tx, row, new_id, false).await?;
        if let Some(cid) = cloud_id {
            cloud_to_local_id.insert(cid, new_id);
        }
        report.new_flights += 1;
    }
    if va_stripped > 0 {
        tracing::info!(
            target: "cloud",
            "download: {va_stripped} vuelo(s) bajados SIN nº de vuelo (mismo nº el mismo día que otro vuelo completado)"
        );
    }

    // Importa solo los tracks de los vuelos NUEVOS — usando el mapping
    // cloud_id → local_id que armamos arriba.
    for track in &remote.flight_log_track {
        let cloud_flight_id = match json_i64(track, "flightId") {
            Some(v) => v,
            None => continue,
        };
        let local_flight_id = match cloud_to_local_id.get(&cloud_flight_id) {
            Some(v) => *v,
            None => continue, // este track corresponde a un vuelo que ya teníamos local
        };
        let ts = json_str(track, "ts").unwrap_or_default();
        if ts.is_empty() {
            continue;
        }
        // (v3.7.0 Phase O) Extendido para los simvars VAS-aligned.
        let b_bool = |key: &str| -> Option<i64> {
            json_i64(track, key).map(|v| if v != 0 { 1 } else { 0 })
        };
        // (v4.0.0 — P4) Extendido con 24 columnas de motor.
        sqlx::query(
            r#"INSERT OR IGNORE INTO flight_log_track (
                flight_id, ts, lat, lon, alt_ft, gs_kt,
                ias_kt, vs_fpm, pitch_deg, bank_deg, g_force,
                flaps_pct, gear_down, spoilers_pct,
                light_nav, light_beacon, light_taxi, light_landing, light_strobe,
                parking_brake, transponder_code,
                eng_n1_1, eng_n1_2, eng_n1_3, eng_n1_4,
                eng_n2_1, eng_n2_2, eng_n2_3, eng_n2_4,
                eng_egt_1, eng_egt_2, eng_egt_3, eng_egt_4,
                eng_ff_pph_1, eng_ff_pph_2, eng_ff_pph_3, eng_ff_pph_4,
                eng_oil_temp_1, eng_oil_temp_2, eng_oil_temp_3, eng_oil_temp_4,
                eng_oil_press_1, eng_oil_press_2, eng_oil_press_3, eng_oil_press_4
            ) VALUES (?,?,?,?,?,?, ?,?,?,?,?, ?,?,?, ?,?,?,?,?, ?,?,
                      ?,?,?,?, ?,?,?,?, ?,?,?,?, ?,?,?,?, ?,?,?,?, ?,?,?,?)"#,
        )
        .bind(local_flight_id)
        .bind(&ts)
        .bind(json_f64(track, "lat").unwrap_or(0.0))
        .bind(json_f64(track, "lon").unwrap_or(0.0))
        .bind(json_i64(track, "altFt"))
        .bind(
            json_i64(track, "gsKt")
                .or_else(|| json_i64(track, "groundSpeedKt")),
        )
        .bind(json_i64(track, "iasKt"))
        .bind(json_i64(track, "vsFpm"))
        .bind(json_f64(track, "pitchDeg"))
        .bind(json_f64(track, "bankDeg"))
        .bind(json_f64(track, "gForce"))
        .bind(json_i64(track, "flapsPct"))
        .bind(b_bool("gearDown"))
        .bind(json_i64(track, "spoilersPct"))
        .bind(b_bool("lightNav"))
        .bind(b_bool("lightBeacon"))
        .bind(b_bool("lightTaxi"))
        .bind(b_bool("lightLanding"))
        .bind(b_bool("lightStrobe"))
        .bind(b_bool("parkingBrake"))
        .bind(json_i64(track, "transponderCode"))
        .bind(json_f64(track, "engN11"))
        .bind(json_f64(track, "engN12"))
        .bind(json_f64(track, "engN13"))
        .bind(json_f64(track, "engN14"))
        .bind(json_f64(track, "engN21"))
        .bind(json_f64(track, "engN22"))
        .bind(json_f64(track, "engN23"))
        .bind(json_f64(track, "engN24"))
        .bind(json_f64(track, "engEgt1"))
        .bind(json_f64(track, "engEgt2"))
        .bind(json_f64(track, "engEgt3"))
        .bind(json_f64(track, "engEgt4"))
        .bind(json_f64(track, "engFfPph1"))
        .bind(json_f64(track, "engFfPph2"))
        .bind(json_f64(track, "engFfPph3"))
        .bind(json_f64(track, "engFfPph4"))
        .bind(json_f64(track, "engOilTemp1"))
        .bind(json_f64(track, "engOilTemp2"))
        .bind(json_f64(track, "engOilTemp3"))
        .bind(json_f64(track, "engOilTemp4"))
        .bind(json_f64(track, "engOilPress1"))
        .bind(json_f64(track, "engOilPress2"))
        .bind(json_f64(track, "engOilPress3"))
        .bind(json_f64(track, "engOilPress4"))
        .execute(&mut *tx)
        .await?;
        report.new_tracks += 1;
    }

    // Settings: siempre se mergean (preserva preferencias del cloud).
    for kv in &remote.settings {
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES (?,?,datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value = excluded.value,
             updated_at = excluded.updated_at",
        )
        .bind(&kv.key)
        .bind(&kv.value)
        .execute(&mut *tx)
        .await?;
        report.new_settings += 1;
    }

    // (v3.6.0 Phase H) Importa score items y phase transitions sólo
    // para los vuelos NUEVOS — mismo mapping cloud_id → local_id que
    // armamos arriba para los tracks.
    for item in &remote.flight_log_score_item {
        let cloud_flight_id = match json_i64(item, "flightId") {
            Some(v) => v,
            None => continue,
        };
        let local_flight_id = match cloud_to_local_id.get(&cloud_flight_id) {
            Some(v) => *v,
            None => continue,
        };
        let Some(phase) = json_str(item, "phase") else { continue };
        let Some(rule_id) = json_str(item, "ruleId") else { continue };
        sqlx::query(
            r#"INSERT OR IGNORE INTO flight_log_score_item
                (flight_id, phase, rule_id, label, points_earned, points_max,
                 passed, severity, evidence, ts)
               VALUES (?,?,?,?,?,?,?,?,?,?)"#,
        )
        .bind(local_flight_id)
        .bind(&phase)
        .bind(&rule_id)
        .bind(json_str(item, "label").unwrap_or_default())
        .bind(json_i64(item, "pointsEarned").unwrap_or(0))
        .bind(json_i64(item, "pointsMax").unwrap_or(0))
        .bind(json_i64(item, "passed").unwrap_or(0))
        .bind(json_str(item, "severity"))
        .bind(json_str(item, "evidence"))
        .bind(json_str(item, "ts"))
        .execute(&mut *tx)
        .await?;
    }
    for ph in &remote.flight_log_phase {
        let cloud_flight_id = match json_i64(ph, "flightId") {
            Some(v) => v,
            None => continue,
        };
        let local_flight_id = match cloud_to_local_id.get(&cloud_flight_id) {
            Some(v) => *v,
            None => continue,
        };
        let Some(phase) = json_str(ph, "phase") else { continue };
        let Some(entered_at) = json_str(ph, "enteredAt") else { continue };
        sqlx::query(
            r#"INSERT OR IGNORE INTO flight_log_phase
                (flight_id, phase, entered_at, exited_at)
               VALUES (?,?,?,?)"#,
        )
        .bind(local_flight_id)
        .bind(&phase)
        .bind(&entered_at)
        .bind(json_str(ph, "exitedAt"))
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    // (24/10) El libro (Tech Log, cobros, GSX…) después de los vuelos: sus
    // registros se enlazan a los vuelos por clave estable.
    let ledger = crate::ledger_sync::apply(pool, &remote.ledger, &tombs).await?;
    report.ledger_changes = ledger.airframes + ledger.mx_flights + ledger.maintenance + ledger.gsx_lines + ledger.other;
    if report.new_flights > 0 || report.deleted_flights > 0 || ledger.any() {
        settle_after_sync(pool).await;
    }
    Ok(report)
}

/// (v7.4.2) INSERT de un vuelo del snapshot en `flight_log` (antes inline en
/// `download_missing`). Incluye flight_number/callsign/airline_icao/status/
/// score_* para preservar la metadata VA sin recomputar. Con
/// `with_number = false` entra SIN flight_number/airline_icao — así no choca
/// con `uq_flight_log_va_dedup_completed` cuando ya hay otro vuelo completed
/// con el mismo nº ese día.
#[allow(clippy::too_many_arguments)]
fn insert_cloud_flight<'q>(
    row: &'q serde_json::Value,
    started_at: &'q str,
    ended_at: Option<&'q str>,
    origin_lat: f64,
    origin_lon: f64,
    source: &'q str,
    external_id: Option<&'q str>,
    with_number: bool,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    let (flight_number, airline_icao) = if with_number {
        (json_str(row, "flightNumber"), json_str(row, "airlineIcao"))
    } else {
        (None, None)
    };
    sqlx::query(
        r#"INSERT INTO flight_log
                (started_at, ended_at, origin_icao, origin_name, origin_lat, origin_lon,
                 destination_icao, destination_name, destination_lat, destination_lon,
                 aircraft_atc_type, aircraft_title,
                 aircraft_model, aircraft_airline, aircraft_registration,
                 distance_nm, flight_time_s,
                 max_altitude_ft, landing_fpm, max_ground_speed_kt, max_true_airspeed_kt,
                 departure_gate, arrival_gate, passengers, cargo_kg, fuel_used_kg,
                 paused_seconds, source, external_id,
                 flight_number, callsign, airline_icao, status,
                 score_total, score_max, score_grade,
                 metar_origin, metar_dest)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"#,
    )
    .bind(started_at)
    .bind(ended_at)
    .bind(json_str(row, "originIcao"))
    .bind(json_str(row, "originName"))
    .bind(origin_lat)
    .bind(origin_lon)
    .bind(json_str(row, "destinationIcao"))
    .bind(json_str(row, "destinationName"))
    .bind(json_f64(row, "destinationLat"))
    .bind(json_f64(row, "destinationLon"))
    .bind(json_str(row, "aircraftAtcType"))
    .bind(json_str(row, "aircraftTitle"))
    .bind(json_str(row, "aircraftModel"))
    .bind(json_str(row, "aircraftAirline"))
    .bind(json_str(row, "aircraftRegistration"))
    .bind(json_f64(row, "distanceNm"))
    .bind(json_i64(row, "flightTimeS"))
    .bind(json_i64(row, "maxAltitudeFt"))
    .bind(json_i64(row, "landingFpm"))
    .bind(json_i64(row, "maxGroundSpeedKt"))
    .bind(json_i64(row, "maxTrueAirspeedKt"))
    .bind(json_str(row, "departureGate"))
    .bind(json_str(row, "arrivalGate"))
    .bind(json_i64(row, "passengers"))
    .bind(json_i64(row, "cargoKg"))
    .bind(json_i64(row, "fuelUsedKg"))
    .bind(json_i64(row, "pausedSeconds").unwrap_or(0))
    .bind(source)
    .bind(external_id)
    .bind(flight_number)
    .bind(json_str(row, "callsign"))
    .bind(airline_icao)
    .bind(json_str(row, "status").unwrap_or_else(|| "completed".to_string()))
    .bind(json_i64(row, "scoreTotal"))
    .bind(json_i64(row, "scoreMax"))
    .bind(json_str(row, "scoreGrade"))
    .bind(json_str(row, "metarOrigin"))
    .bind(json_str(row, "metarDest"))
}

/// (v7.4.2) Claves del índice `uq_flight_log_va_dedup_completed` de los
/// vuelos LOCALES: `airline|flight_number|date(started_at)` de los completed.
async fn build_local_va_keys(
    pool: &SqlitePool,
) -> anyhow::Result<std::collections::HashSet<String>> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT airline_icao, flight_number, date(started_at) FROM flight_log \
         WHERE airline_icao IS NOT NULL AND flight_number IS NOT NULL AND status = 'completed'",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(a, n, d)| d.map(|d| format!("{a}|{n}|{d}")))
        .collect())
}

/// (v7.4.2) Clave del índice VA de un vuelo del snapshot (`None` si el índice
/// no le aplica). El día se calcula con el `date()` de SQLite — exactamente
/// la misma expresión que usa el índice, también con sufijos `Z`/offset.
async fn cloud_va_key(
    pool: &SqlitePool,
    row: &serde_json::Value,
) -> anyhow::Result<Option<String>> {
    let status = json_str(row, "status").unwrap_or_else(|| "completed".to_string());
    let (Some(airline), Some(number)) = (json_str(row, "airlineIcao"), json_str(row, "flightNumber"))
    else {
        return Ok(None);
    };
    if status != "completed" {
        return Ok(None);
    }
    let started = json_str(row, "startedAt").unwrap_or_default();
    let day: Option<String> = sqlx::query_scalar("SELECT date(?)")
        .bind(&started)
        .fetch_one(pool)
        .await?;
    Ok(day.map(|d| format!("{airline}|{number}|{d}")))
}

/// Construye un set de claves estables para los vuelos LOCALES, para
/// hacer dedup contra los del cloud durante `download_missing`. Cada
/// vuelo aporta hasta DOS claves:
///   - `source|external_id` (si tiene external_id — todos los imports)
///   - `started_at|origin_lat:2|origin_lon:2` (sim-flown sin external_id)
async fn build_local_dedup_keys(
    pool: &SqlitePool,
) -> anyhow::Result<std::collections::HashSet<String>> {
    let rows: Vec<(String, Option<String>, String, f64, f64)> = sqlx::query_as(
        "SELECT source, external_id, started_at, origin_lat, origin_lon FROM flight_log",
    )
    .fetch_all(pool)
    .await?;
    let mut set = std::collections::HashSet::new();
    for (source, ext, started, lat, lon) in rows {
        if let Some(eid) = ext {
            set.insert(format!("{}|{}", source, eid));
        }
        set.insert(format!("{}|{:.2}|{:.2}", started, lat, lon));
    }
    Ok(set)
}

// =============================================================================
// Folder sync (v2.0.1) — alternativa simple a OAuth
// =============================================================================

/// Estado del folder sync — el frontend lo lee para mostrar el path
/// configurado y el último sync.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSyncConfig {
    /// Path absoluto al folder que el usuario eligió. None si nunca
    /// configuró uno. El folder NO tiene que existir todavía (se crea
    /// al save) — útil cuando OneDrive aún no ha bajado la carpeta.
    pub folder_path: Option<String>,
    /// Timestamp del último save exitoso (UTC ISO).
    pub last_sync_at: Option<String>,
    /// True si el path existe en disco AHORA (puede haber sido movido).
    pub folder_exists: bool,
    /// True si el archivo `msfs-addons-data.json` ya existe ahí — útil
    /// para distinguir "primer save" vs "ya hay un backup que restaurar".
    pub data_file_exists: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSyncSaveReport {
    /// Ruta absoluta donde se escribió el archivo.
    pub output_path: String,
    pub bytes_written: u64,
    pub flights: usize,
    pub tracks: usize,
    pub settings: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSyncLoadReport {
    pub source_path: String,
    pub bytes_read: u64,
    pub restored_flights: usize,
    pub restored_tracks: usize,
    pub restored_settings: usize,
}

pub async fn get_folder_config(pool: &SqlitePool) -> anyhow::Result<FolderSyncConfig> {
    let path_opt = get_setting(pool, KEY_FOLDER_SYNC_PATH).await?;
    let last_at = get_setting(pool, KEY_FOLDER_SYNC_LAST_AT).await?;
    let folder_exists = path_opt
        .as_deref()
        .map(|p| std::path::Path::new(p).is_dir())
        .unwrap_or(false);
    let data_file_exists = path_opt
        .as_deref()
        .map(|p| std::path::Path::new(p).join(crate::test_env::sync_download_file_name()).is_file())
        .unwrap_or(false);
    Ok(FolderSyncConfig {
        folder_path: path_opt,
        last_sync_at: last_at,
        folder_exists,
        data_file_exists,
    })
}

pub async fn set_folder_path(
    pool: &SqlitePool,
    folder_path: &str,
) -> anyhow::Result<()> {
    let trimmed = folder_path.trim();
    if trimmed.is_empty() {
        delete_setting(pool, KEY_FOLDER_SYNC_PATH).await?;
    } else {
        set_setting(pool, KEY_FOLDER_SYNC_PATH, trimmed).await?;
    }
    Ok(())
}

/// Escribe el snapshot completo a `<folder>/msfs-addons-data.json`.
/// El cliente de OneDrive/Drive Desktop/Dropbox se encargará de
/// subirlo a la nube. Si el folder no existe, lo crea (caso OneDrive
/// con carpeta no sincronizada aún).
pub async fn save_to_folder(
    pool: &SqlitePool,
    folder_path: &str,
) -> anyhow::Result<FolderSyncSaveReport> {
    let snap = build_snapshot(pool).await?;
    let payload = serde_json::to_string_pretty(&snap)?;
    let path = std::path::PathBuf::from(folder_path);
    tokio::fs::create_dir_all(&path)
        .await
        .with_context(|| crate::tr!("cloudSync.folderCreateFailed", path = path.display()))?;
    let out = path.join(crate::test_env::sync_upload_file_name());
    let bytes_written = payload.len() as u64;
    tokio::fs::write(&out, payload)
        .await
        .with_context(|| crate::tr!("cloudSync.folderWriteFailed", path = out.display()))?;
    // Persistimos el path elegido para próximos saves automáticos.
    set_setting(pool, KEY_FOLDER_SYNC_PATH, folder_path.trim()).await?;
    set_setting(
        pool,
        KEY_FOLDER_SYNC_LAST_AT,
        &chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    )
    .await?;
    Ok(FolderSyncSaveReport {
        output_path: out.to_string_lossy().into_owned(),
        bytes_written,
        flights: snap.flight_log.len(),
        tracks: snap.flight_log_track.len(),
        settings: snap.settings.len(),
    })
}

/// Lee `<folder>/msfs-addons-data.json` y mergea con la DB local.
/// Reglas de merge: id-based upsert para flight_log, (flight_id, ts)
/// INSERT-OR-IGNORE para track points, key-based upsert para settings.
/// Si el archivo no existe, devuelve error claro.
pub async fn load_from_folder(
    pool: &SqlitePool,
    folder_path: &str,
) -> anyhow::Result<FolderSyncLoadReport> {
    let path = std::path::PathBuf::from(folder_path).join(crate::test_env::sync_download_file_name());
    if !path.is_file() {
        return Err(anyhow!("{}", crate::tr!("cloudSync.folderBackupMissing", path = path.display())));
    }
    let text = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| crate::tr!("cloudSync.folderReadFailed", path = path.display()))?;
    let bytes_read = text.len() as u64;
    let snap: Snapshot = serde_json::from_str(&text)
        .with_context(|| crate::tr!("cloudSync.fileCorruptOrIncompatible"))?;
    // (v7 fix, pérdida de datos) `restore_snapshot` hace upsert POR id, y el id
    // de flight_log es un autoincrement LOCAL: el id=5 de esta PC y el id=5 del
    // backup son vuelos DISTINTOS → restaurar por id sobre una DB con datos
    // PISARÍA vuelos locales. Sólo es seguro en una DB vacía (reinstalación). Si
    // ya hay vuelos, no arriesgamos. (v7.4.2) En vez de negarnos, usamos el
    // mismo merge por CLAVES estables que "Bajar" (`merge_remote_snapshot`):
    // añade sólo lo que falta y nunca pisa ni borra vuelos locales.
    let local_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM flight_log")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let restored = if local_count > 0 {
        let r = merge_remote_snapshot(pool, &snap).await?;
        RestoreReport {
            flights: r.new_flights,
            tracks: r.new_tracks,
            settings: r.new_settings,
        }
    } else {
        restore_snapshot(pool, &snap).await?
    };
    // Marcamos el path como activo y registramos last sync (incluso si
    // fue un load, el merge es bidireccional desde el punto de vista
    // del usuario: "la app ahora tiene los datos de Drive").
    set_setting(pool, KEY_FOLDER_SYNC_PATH, folder_path.trim()).await?;
    set_setting(
        pool,
        KEY_FOLDER_SYNC_LAST_AT,
        &chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    )
    .await?;
    Ok(FolderSyncLoadReport {
        source_path: path.to_string_lossy().into_owned(),
        bytes_read,
        restored_flights: restored.flights,
        restored_tracks: restored.tracks,
        restored_settings: restored.settings,
    })
}

// =============================================================================
// Crypto helpers — sin nuevas crates
// =============================================================================

fn random_url_safe_string(len: usize) -> String {
    use base64::Engine;
    // Mezclamos UUID v4 (en deps existentes) varias veces hasta llenar
    // `len` bytes — UUID v4 da 16 bytes pseudo-random por iteración.
    let mut bytes = Vec::with_capacity(len);
    while bytes.len() < len {
        bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    bytes.truncate(len);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
}

fn sha256(input: &[u8]) -> [u8; 32] {
    sha256_impl::hash(input)
}

fn base64_urlsafe_no_pad(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

mod sha256_impl {
    // SHA-256 puro Rust, ~80 líneas. Suficiente para 32-byte hashes
    // de un code_verifier. Evita arrastrar `sha2` por sólo el PKCE
    // challenge.
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    const H0: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    pub fn hash(input: &[u8]) -> [u8; 32] {
        let mut msg = input.to_vec();
        let bit_len = (input.len() as u64).wrapping_mul(8);
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bit_len.to_be_bytes());
        let mut h = H0;
        for chunk in msg.chunks(64) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([
                    chunk[i * 4],
                    chunk[i * 4 + 1],
                    chunk[i * 4 + 2],
                    chunk[i * 4 + 3],
                ]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7)
                    ^ w[i - 15].rotate_right(18)
                    ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17)
                    ^ w[i - 2].rotate_right(19)
                    ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let mut a = h[0];
            let mut b = h[1];
            let mut c = h[2];
            let mut d = h[3];
            let mut e = h[4];
            let mut f = h[5];
            let mut g = h[6];
            let mut hh = h[7];
            for i in 0..64 {
                let s1 =
                    e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ (!e & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 =
                    a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
            h[5] = h[5].wrapping_add(f);
            h[6] = h[6].wrapping_add(g);
            h[7] = h[7].wrapping_add(hh);
        }
        let mut out = [0u8; 32];
        for i in 0..8 {
            out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
        }
        out
    }
}

#[cfg(test)]
mod download_merge_tests {
    use super::*;
    use serde_json::json;

    async fn mem_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn flight(id: i64, started: &str, lat: f64, time_s: i64, airline: Option<&str>, number: Option<&str>) -> serde_json::Value {
        json!({
            "id": id, "startedAt": started, "endedAt": started, "originLat": lat, "originLon": -70.0,
            "flightTimeS": time_s, "airlineIcao": airline, "flightNumber": number,
            "callsign": number.map(|n| format!("{}{}", airline.unwrap_or(""), n)),
            "status": "completed", "source": "simconnect"
        })
    }

    fn snapshot(flights: Vec<serde_json::Value>) -> Snapshot {
        serde_json::from_value(json!({
            "version": 1, "exportedAt": "2026-08-27T00:00:00Z",
            "flightLog": flights, "flightLogTrack": [], "settings": []
        }))
        .unwrap()
    }

    /// Reproduce el error del usuario: mismo nº de vuelo el mismo día (vuelo
    /// real + sesiones cortas), un vuelo repetido dentro del snapshot y un
    /// choque con un vuelo LOCAL. Antes: "UNIQUE constraint failed: index
    /// 'uq_flight_log_va_dedup_completed'" y no bajaba nada.
    #[tokio::test]
    async fn same_flight_number_same_day_does_not_abort_download() {
        let pool = mem_pool().await;
        sqlx::query(
            "INSERT INTO flight_log (started_at, origin_lat, origin_lon, airline_icao, flight_number, status, source) \
             VALUES ('2026-06-27T18:13:17Z', 29.9, -95.3, 'DAL', '1069', 'completed', 'simconnect')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let snap = snapshot(vec![
            flight(157, "2026-06-18T22:50:04Z", 18.4, 1604, Some("DAL"), Some("384")),
            flight(158, "2026-06-18T23:58:38Z", 42.3, 22232, Some("DAL"), Some("384")),
            flight(157, "2026-06-18T22:50:04Z", 18.4, 1604, Some("DAL"), Some("384")), // repetido
            flight(169, "2026-06-27T16:08:52Z", 29.9, 120, Some("DAL"), Some("1069")), // choca con local
            flight(200, "2026-07-01T10:00:00Z", 10.0, 3600, None, None),
        ]);
        let report = merge_remote_snapshot(&pool, &snap).await.expect("la bajada no debe abortar");
        assert_eq!(report.new_flights, 4);
        assert_eq!(report.skipped_existing, 1);

        // El nº lo conserva el vuelo MÁS LARGO; la sesión corta entra sin nº.
        let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT started_at, flight_number, callsign FROM flight_log ORDER BY started_at",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let by_start = |s: &str| rows.iter().find(|r| r.0 == s).cloned().unwrap();
        assert_eq!(by_start("2026-06-18T23:58:38Z").1.as_deref(), Some("384"));
        assert_eq!(by_start("2026-06-18T22:50:04Z").1, None);
        assert_eq!(by_start("2026-06-18T22:50:04Z").2.as_deref(), Some("DAL384"), "el callsign se conserva");
        assert_eq!(by_start("2026-06-27T16:08:52Z").1, None, "el local conserva su nº");

        // Idempotente: bajar otra vez no inserta nada.
        let again = merge_remote_snapshot(&pool, &snap).await.unwrap();
        assert_eq!(again.new_flights, 0);
    }

    /// El auto-sync con la DB VACÍA usa `restore_snapshot` (upsert por id) —
    /// mismo choque, mismo arreglo.
    #[tokio::test]
    async fn restore_on_empty_db_survives_same_flight_number() {
        let pool = mem_pool().await;
        let snap = snapshot(vec![
            flight(174, "2026-06-29T15:49:23Z", 33.6, 2799, Some("DAL"), Some("1905")),
            flight(175, "2026-06-29T17:07:27Z", 33.7, 179, Some("DAL"), Some("1905")),
            flight(176, "2026-06-29T20:24:54Z", 33.8, 1842, Some("DAL"), Some("1905")),
        ]);
        let r = restore_snapshot(&pool, &snap).await.expect("el restore no debe abortar");
        assert_eq!(r.flights, 3);
        let kept: Vec<(i64,)> = sqlx::query_as("SELECT id FROM flight_log WHERE flight_number = '1905'")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(kept, vec![(174,)], "sólo el más largo conserva el nº");
    }

    /// Repetición con un snapshot REAL (no corre por defecto):
    ///   SIMFLEET_SNAPSHOT=<json> [SIMFLEET_DB_COPY=<copia .db>] cargo test --lib real_snapshot -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn real_snapshot_merges_cleanly() {
        let Ok(path) = std::env::var("SIMFLEET_SNAPSHOT") else { return };
        let snap: Snapshot = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let pool = match std::env::var("SIMFLEET_DB_COPY") {
            Ok(db) => {
                let pool = SqlitePool::connect(&format!("sqlite://{db}")).await.unwrap();
                sqlx::migrate!("./migrations").run(&pool).await.unwrap();
                pool
            }
            Err(_) => mem_pool().await,
        };
        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM flight_log").fetch_one(&pool).await.unwrap();
        let r = merge_remote_snapshot(&pool, &snap).await.expect("merge del snapshot real");
        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM flight_log").fetch_one(&pool).await.unwrap();
        println!(
            "snapshot real: cloud={} nuevos={} omitidos={} tracks={} · vuelos en DB {} → {}",
            r.cloud_flights, r.new_flights, r.skipped_existing, r.new_tracks, before, after
        );
        let again = merge_remote_snapshot(&pool, &snap).await.unwrap();
        println!("segunda bajada: nuevos={}", again.new_flights);
        assert_eq!(again.new_flights, 0);
        // Y el camino del auto-sync con DB vacía (restore por id).
        let empty = mem_pool().await;
        let rr = restore_snapshot(&empty, &snap).await.expect("restore del snapshot real");
        println!("restore en DB vacía: vuelos={} tracks={}", rr.flights, rr.tracks);
    }
}

#[cfg(test)]
mod ledger_cloud_tests {
    //! (24/10) Dos PCs y la nube, sin red: el Tech Log y las finanzas viajan,
    //! lo borrado en una PC se borra en la otra con todo lo suyo y no vuelve,
    //! y sin cambios no se re-sube.
    use super::*;
    use crate::ledger_sync::{self, KIND_MAINT};
    use crate::mx::recorder::{FlightRecord, LandingRec};

    async fn pc() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// La nube guarda JSON: lo que baja es lo que se serializó.
    fn via_cloud(s: &Snapshot) -> Snapshot {
        serde_json::from_str(&serde_json::to_string(s).unwrap()).unwrap()
    }

    async fn fly(pool: &SqlitePool, started: &str, sink_fpm: f64) -> i64 {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO flight_log (started_at, ended_at, origin_lat, origin_lon, origin_icao, destination_icao,
                                     aircraft_title, aircraft_atc_type, aircraft_registration, status, source, flight_time_s)
             VALUES (?1, ?1, 40.47, -3.56, 'LEMD', 'LEBL', 'iFly 737-MAX8 Test', 'B38M', 'EC-TST', 'completed', 'simconnect', 3600)
             RETURNING id",
        )
        .bind(started)
        .fetch_one(pool)
        .await
        .unwrap();
        let rec = FlightRecord {
            flight_id: id,
            block_h: 1.0,
            air_h: 0.8,
            eng_h: [1.0, 1.0],
            eng_starts: [1, 1],
            eng_ran: [true, true],
            takeoffs: 1,
            landings: vec![LandingRec { sink_fpm, peak_g: 1.3, weight_kg: 60_000.0, gs_kt: 135.0, ..Default::default() }],
            ..Default::default()
        };
        crate::mx::store::commit_flight(pool, rec, Some(-(sink_fpm as i64))).await.unwrap();
        id
    }

    /// Tech Log del avión sin ids (cada PC tiene los suyos). A 6 decimales: el
    /// JSON de la nube puede mover el último bit de un REAL.
    async fn techlog(pool: &SqlitePool) -> (Vec<(String, String, i64, f64, f64, String)>, Vec<(String, String)>, String) {
        let comps = sqlx::query_as("SELECT key, position, serial, ROUND(wear, 6), ROUND(basis_at_install, 6), status FROM mx_component ORDER BY key, position")
            .fetch_all(pool)
            .await
            .unwrap();
        let defects = sqlx::query_as("SELECT title, status FROM mx_defect ORDER BY title, status")
            .fetch_all(pool)
            .await
            .unwrap();
        let counters = sqlx::query_scalar("SELECT counters FROM mx_airframe").fetch_one(pool).await.unwrap();
        (comps, defects, counters)
    }

    async fn count(pool: &SqlitePool, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
    }

    #[tokio::test]
    async fn two_pcs_share_the_ledger_and_deletions() {
        // PC A: dos vuelos (el primero, aterrizaje duro), un cobro y una política.
        let a = pc().await;
        let f1 = fly(&a, "2026-10-20T10:00:00Z", 720.0).await;
        fly(&a, "2026-10-20T14:00:00Z", 150.0).await;
        sqlx::query("INSERT INTO maintenance_log (registration, component, cost, serviced_at) VALUES ('ECTST', 'mx:part_repair:Brakes', 900, '2026-10-20T16:00:00Z')")
            .execute(&a)
            .await
            .unwrap();
        sqlx::query("INSERT INTO maintenance_log (registration, component, cost, serviced_at) VALUES ('ECTST', 'catering', 58, '2026-10-20T17:00:00Z')")
            .execute(&a)
            .await
            .unwrap();
        sqlx::query("INSERT INTO airline_policy (airline_key, maintenance_level, updated_at) VALUES ('IBE', 2, '2026-10-20T00:00:00Z')")
            .execute(&a)
            .await
            .unwrap();
        assert_eq!(count(&a, "SELECT COUNT(*) FROM mx_flight").await, 2);
        crate::airline_economy::recompute(&a).await.unwrap(); // como al cerrar un vuelo

        // A sube a una nube vacía; volver a subir sin cambios no sube nada.
        let up = merge_for_upload(&a, Snapshot::default()).await.unwrap();
        assert!(up.changed);
        let cloud1 = via_cloud(&up.snapshot);
        assert_eq!(cloud1.flight_log.len(), 2);
        assert_eq!(cloud1.ledger.airframes.len(), 1);
        assert_eq!(cloud1.ledger.mx_flights.len(), 2);
        assert_eq!(cloud1.ledger.maintenance_log.len(), 2);
        assert!(!merge_for_upload(&a, via_cloud(&cloud1)).await.unwrap().changed, "A sin cambios no re-sube");

        // PC B (nueva): recibe vuelos, Tech Log, cobros y política.
        let b = pc().await;
        restore_snapshot(&b, &cloud1).await.unwrap();
        assert_eq!(count(&b, "SELECT COUNT(*) FROM flight_log").await, 2);
        assert_eq!(count(&b, "SELECT COUNT(*) FROM mx_flight").await, 2);
        assert_eq!(techlog(&b).await, techlog(&a).await, "mismo Tech Log en las dos PCs");
        assert_eq!(count(&b, "SELECT COUNT(*) FROM maintenance_log").await, 2);
        assert_eq!(count(&b, "SELECT maintenance_level FROM airline_policy WHERE airline_key = 'IBE'").await, 2);
        assert!(!merge_for_upload(&b, via_cloud(&cloud1)).await.unwrap().changed, "B sin cambios no re-sube");

        // A borra el vuelo 1 (y un cobro doble): la nube los pierde.
        let before_f1 = techlog(&a).await;
        crate::flight_log::delete_entry(&a, f1).await.unwrap();
        assert_ne!(techlog(&a).await, before_f1, "el Tech Log del vuelo 1 se revirtió en A");
        sqlx::query("DELETE FROM maintenance_log WHERE component = 'catering'").execute(&a).await.unwrap();
        ledger_sync::record(&a, KIND_MAINT, &ledger_sync::maint_key("ECTST", "catering", "2026-10-20T17:00:00Z"))
            .await
            .unwrap();
        let up2 = merge_for_upload(&a, via_cloud(&cloud1)).await.unwrap();
        assert!(up2.changed);
        assert_eq!(up2.removed_flights, 1);
        let cloud2 = via_cloud(&up2.snapshot);
        assert_eq!(cloud2.flight_log.len(), 1);
        assert_eq!(cloud2.ledger.mx_flights.len(), 1);
        assert_eq!(cloud2.ledger.maintenance_log.len(), 1);

        // B baja: el vuelo 1 se borra también allí, con su Tech Log y su cobro.
        let rep = merge_remote_snapshot(&b, &cloud2).await.unwrap();
        assert_eq!(rep.deleted_flights, 1);
        assert_eq!(rep.new_flights, 0);
        assert_eq!(count(&b, "SELECT COUNT(*) FROM flight_log").await, 1);
        assert_eq!(count(&b, "SELECT COUNT(*) FROM mx_flight").await, 1);
        assert_eq!(count(&b, "SELECT COUNT(*) FROM maintenance_log").await, 1);
        assert_eq!(techlog(&b).await, techlog(&a).await, "B quedó igual que A");

        // Y no vuelve: ni al bajar otra vez, ni al subir B, ni desde la nube vieja.
        let again = merge_remote_snapshot(&b, &cloud2).await.unwrap();
        assert_eq!((again.new_flights, again.deleted_flights), (0, 0));
        assert!(!merge_for_upload(&b, via_cloud(&cloud2)).await.unwrap().changed, "B sin cambios no re-sube");
        let stale = merge_for_upload(&b, via_cloud(&cloud1)).await.unwrap();
        assert_eq!(stale.snapshot.flight_log.len(), 1, "una nube vieja (otra versión) no lo resucita");
        let old = merge_remote_snapshot(&b, &cloud1).await.unwrap();
        assert_eq!(old.new_flights, 0, "bajar una nube vieja tampoco");
    }

    /// Con una COPIA real (no corre por defecto):
    ///   LEDGER_DB=<copia .db> cargo test --lib ledger_real_copy -- --ignored --nocapture
    /// Sube todo desde la copia, lo restaura en una PC vacía y compara.
    #[tokio::test]
    #[ignore]
    async fn ledger_real_copy() {
        let Ok(path) = std::env::var("LEDGER_DB") else { return };
        let a = SqlitePool::connect(&format!("sqlite:{path}")).await.unwrap();
        sqlx::migrate!("./migrations").run(&a).await.unwrap();
        crate::airline_economy::recompute(&a).await.unwrap(); // P&L con esta versión
        let up = merge_for_upload(&a, Snapshot::default()).await.unwrap();
        let l = &up.snapshot.ledger;
        println!(
            "libro: {} aviones · {} registros de vuelo · {} cobros · {} vistos · {} líneas GSX · {} políticas · {} traslados · {} reservas · {} días · {} ajustes",
            l.airframes.len(), l.mx_flights.len(), l.maintenance_log.len(), l.techlog_seen.len(), l.gsx_lines.len(),
            l.policies.len(), l.ferries.len(), l.reservations.len(), l.auto_daily.len(), l.settings.len()
        );
        println!(
            "JSON: libro {:.0} KB · snapshot entero {:.1} MB · borrados {}",
            serde_json::to_string(&up.snapshot.ledger).unwrap().len() as f64 / 1024.0,
            serde_json::to_string(&up.snapshot).unwrap().len() as f64 / 1_048_576.0,
            up.snapshot.deleted.len()
        );
        assert!(!merge_for_upload(&a, via_cloud(&up.snapshot)).await.unwrap().changed, "sin cambios no re-sube");

        let cloud = via_cloud(&up.snapshot);
        let b = pc().await;
        restore_snapshot(&b, &cloud).await.unwrap();
        for (what, sql) in [
            ("vuelos", "SELECT COUNT(*) FROM flight_log"),
            ("aviones", "SELECT COUNT(*) FROM mx_airframe"),
            ("piezas", "SELECT COUNT(*) FROM mx_component"),
            ("defectos", "SELECT COUNT(*) FROM mx_defect"),
            ("registros de vuelo", "SELECT COUNT(*) FROM mx_flight"),
            ("cobros", "SELECT COUNT(*) FROM maintenance_log"),
            ("líneas GSX cobradas", "SELECT COUNT(*) FROM gsx_charge WHERE flight_id IS NOT NULL"),
            ("P&L", "SELECT COUNT(*) FROM flight_pnl"),
        ] {
            let (x, y) = (count(&a, sql).await, count(&b, sql).await);
            println!("{what:>22}: original {x:>6} · PC nueva {y:>6}");
        }
        // Diferencias de P&L por vuelo (clave estable: cada PC tiene sus ids).
        let pnl = |pool: SqlitePool| async move {
            let rows = sqlx::query(
                "SELECT p.*, f.started_at || '|' || printf('%.2f', f.origin_lat) || '|' || printf('%.2f', f.origin_lon) AS fkey
                   FROM flight_pnl p JOIN flight_log f ON f.id = p.flight_id",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
            rows.iter().map(row_to_json).collect::<Vec<_>>()
        };
        let (pa, pb) = (pnl(a.clone()).await, pnl(b.clone()).await);
        let mut diffs = 0;
        for x in &pa {
            let k = x.get("fkey").cloned();
            let Some(y) = pb.iter().find(|y| y.get("fkey").cloned() == k) else {
                println!("P&L sólo en original: {k:?}");
                diffs += 1;
                continue;
            };
            for (f, v) in x.as_object().unwrap() {
                if y.get(f) != Some(v) && !matches!(f.as_str(), "flightId" | "createdAt" | "computedAt") {
                    println!("{k:?} · {f}: original {v} · PC nueva {}", y.get(f).unwrap_or(&serde_json::Value::Null));
                    diffs += 1;
                }
            }
        }
        println!("diferencias de P&L: {diffs}");
        let money = "SELECT CAST(ROUND(SUM(net)) AS INTEGER) FROM flight_pnl";
        println!("{:>22}: original {:>6} · PC nueva {:>6}", "neto de vuelos (USD)", count(&a, money).await, count(&b, money).await);
        assert!(!merge_for_upload(&b, via_cloud(&cloud)).await.unwrap().changed, "la PC nueva no re-sube");
    }

    /// Borrado y luego reimportado a propósito: vuelve en todas las PCs.
    #[tokio::test]
    async fn a_reimported_flight_comes_back_everywhere() {
        let flights = "SELECT COUNT(*) FROM flight_log";
        let a = pc().await;
        let f1 = fly(&a, "2026-10-20T10:00:00Z", 150.0).await;
        let cloud1 = via_cloud(&merge_for_upload(&a, Snapshot::default()).await.unwrap().snapshot);
        let b = pc().await;
        restore_snapshot(&b, &cloud1).await.unwrap();
        crate::flight_log::delete_entry(&a, f1).await.unwrap();
        let cloud2 = via_cloud(&merge_for_upload(&a, cloud1).await.unwrap().snapshot);
        merge_remote_snapshot(&b, &cloud2).await.unwrap();
        assert_eq!(count(&b, flights).await, 0);

        fly(&a, "2026-10-20T10:00:00Z", 150.0).await;
        let cloud3 = via_cloud(&merge_for_upload(&a, cloud2).await.unwrap().snapshot);
        assert_eq!(cloud3.flight_log.len(), 1);
        assert!(cloud3.deleted.iter().all(|t| !t.active()));
        let rep = merge_remote_snapshot(&b, &cloud3).await.unwrap();
        assert_eq!(rep.new_flights, 1);
        assert_eq!(count(&b, flights).await, 1);
    }
}
