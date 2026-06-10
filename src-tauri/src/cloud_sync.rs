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
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const SYNC_FILE_NAME: &str = "msfs-addons-data.json";

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

/// Tamaño de chunk para el stream de subida — 256 KiB es un buen balance
/// entre granularidad de la barra y overhead por chunk.
const TRANSFER_CHUNK: usize = 256 * 1024;

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
/// Construye un `reqwest::Body` por streaming a partir de `bytes`, emitiendo
/// progreso de subida a medida que cada chunk se entrega al transporte. El
/// llamador DEBE fijar `Content-Length = total` para evitar chunked-encoding
/// y mantener el formato idéntico al buffered.
fn counting_upload_body(
    bytes: Vec<u8>,
    total: u64,
    app: Option<AppHandle>,
) -> reqwest::Body {
    // Partimos en chunks owned para poder mover cada uno al stream.
    let chunks: Vec<Vec<u8>> = bytes
        .chunks(TRANSFER_CHUNK)
        .map(|c| c.to_vec())
        .collect();
    let mut sent: u64 = 0;
    let stream = futures_util::stream::iter(chunks).map(move |chunk| {
        sent += chunk.len() as u64;
        emit_transfer_progress(app.as_ref(), "upload", sent, total, false);
        Ok::<Vec<u8>, std::io::Error>(chunk)
    });
    reqwest::Body::wrap_stream(stream)
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

/// (v3.1.0 / v3.1.1) Lista blanca de emails Gmail autorizados a hacer
/// sync. La app rechaza el OAuth si el email del usuario no está
/// aquí. Hardcodeado por diseño: ESTE NO ES SOFTWARE PÚBLICO — son
/// dos usuarios. Cambiar la lista requiere recompilar.
const WHITELIST_EMAILS: &[&str] = &[
    "hectorvelez1012@gmail.com",
    "jose.daniel0318@gmail.com",
];

const KEY_CLIENT_ID: &str = "google_client_id";
const KEY_CLIENT_SECRET: &str = "google_client_secret";
const KEY_REFRESH_TOKEN: &str = "google_refresh_token";
const KEY_USER_EMAIL: &str = "google_user_email";
const KEY_LAST_SYNC_AT: &str = "google_last_sync_at";

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

/// (v3.1.0) Verifica que el email esté en la whitelist. Si la lista
/// está vacía, lo loguea como warning y rechaza por seguridad
/// (better safe than allowing all).
fn is_whitelisted(email: &str) -> bool {
    let lower = email.trim().to_lowercase();
    if WHITELIST_EMAILS.is_empty() {
        tracing::warn!(
            target: "cloud",
            "WHITELIST_EMAILS está vacía — rechazando '{}'. Recompila con la lista de emails autorizados.",
            email
        );
        return false;
    }
    WHITELIST_EMAILS
        .iter()
        .any(|w| w.eq_ignore_ascii_case(&lower))
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
    Ok(CloudConfig {
        connected,
        has_credentials,
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
        .ok_or_else(|| anyhow!(
            "Las credenciales de Google no están embebidas en esta build. \
             Si ves este mensaje en producción, contacta al desarrollador — \
             la app debió compilarse con SIMFLEET_GOOGLE_CLIENT_ID/SECRET."
        ))?;
    let client_secret = client_secret_opt
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!(
            "Las credenciales de Google no están embebidas en esta build."
        ))?;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("No se pudo abrir el listener local para el callback OAuth")?;
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
        _ = &mut timeout => return Err(anyhow!("Timeout esperando autorización (5 min)")),
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
        "<!doctype html><html><body style='font-family:system-ui;padding:32px;background:#0f172a;color:#e2e8f0'><h2>❌ Error en la autorización</h2><p>Vuelve a la app e inténtalo de nuevo.</p></body></html>"
    } else {
        "<!doctype html><html><body style='font-family:system-ui;padding:32px;background:#0f172a;color:#e2e8f0'><h2>✓ Conectado correctamente</h2><p>Puedes cerrar esta pestaña y volver a la app.</p></body></html>"
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;

    if !success {
        return Err(anyhow!("Callback inválido (state o code faltantes)"));
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
        anyhow!("Google no devolvió refresh_token. Re-intenta con la app de OAuth en modo 'Desktop' y consent screen aprobado.")
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
            return Err(anyhow!(
                "El email '{}' no está autorizado para sincronizar. \
                 Contacta al desarrollador para añadirlo a la whitelist.",
                e
            ));
        }
    } else {
        return Err(anyhow!(
            "No se pudo obtener el email del usuario para validar la whitelist."
        ));
    }

    set_setting(&pool, KEY_REFRESH_TOKEN, &refresh_token).await?;
    if let Some(e) = &email {
        set_setting(&pool, KEY_USER_EMAIL, e).await?;
    }
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
        return Err(anyhow!(
            "Google /token devolvió {}: {}",
            status,
            txt
        ));
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
        .ok_or_else(|| anyhow!("No conectado — pulsa Conectar primero"))?;
    // (v3.6.3 fix I13) Usar resolve_credentials para que mire los valores
    // hardcoded del binario primero (build.rs + secrets.local.toml) y
    // SOLO si faltan caiga al fallback DB. Antes leía solo DB y daba
    // "Client ID no configurado" en step 3 del diagnostic aunque las
    // credenciales estuvieran embedidas correctamente.
    let (cid_opt, secret_opt) = resolve_credentials(pool).await?;
    let client_id = cid_opt
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Client ID no embebido en el binario (re-buildea con secrets.local.toml)"))?;
    let client_secret = secret_opt
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Client Secret no embebido en el binario"))?;
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
        return Err(anyhow!(
            "Refresh token rechazado ({}): {}. Vuelve a Conectar.",
            status,
            txt
        ));
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
        401 => anyhow!(
            "Google Drive 401 (no autorizado). El access_token expiró o las \
             credenciales son inválidas. Pulsa Desconectar y vuelve a Conectar. \
             Detalle: {body}"
        ),
        403 => anyhow!(
            "Google Drive 403 (prohibido). Lo más común: no activaste la Google \
             Drive API en tu proyecto de Google Cloud. Ábrela en \
             https://console.cloud.google.com/apis/library/drive.googleapis.com \
             (selecciona TU proyecto, arriba) y pulsa 'Enable'. También puede ser \
             cuota del proyecto excedida. Detalle: {body}"
        ),
        404 => anyhow!(
            "Google Drive 404 (no encontrado). El archivo de sync remoto ya no \
             existe (se borró o se purgó). Reintenta: se recreará solo. \
             Detalle: {body}"
        ),
        429 => anyhow!(
            "Google Drive 429 (demasiadas peticiones). Rate-limit/cuota excedido. \
             Espera un momento y reintenta. Detalle: {body}"
        ),
        500..=599 => anyhow!(
            "Google Drive {code} (error del servidor de Google — no es tu \
             configuración). Reintenta en unos minutos. Detalle: {body}"
        ),
        _ => anyhow!("Google Drive {status}: {body}"),
    }
}

/// (v3.38.0 #12) Clasifica un error de TRANSPORTE de reqwest (ocurre
/// ANTES de recibir una respuesta HTTP): timeout, conexión/DNS, TLS…
/// en un mensaje accionable. Para errores CON respuesta se usa
/// `drive_error_for_response`.
fn transport_error(context: &str, e: reqwest::Error) -> anyhow::Error {
    let kind = if e.is_timeout() {
        "timeout — Google tardó demasiado en responder (revisa tu internet, o un \
         firewall/proxy/antivirus que esté bloqueando la conexión)"
    } else if e.is_connect() {
        "no se pudo conectar con Google (sin internet, fallo de DNS, o un \
         firewall/proxy bloqueando *.googleapis.com)"
    } else if e.is_request() {
        "no se pudo construir/enviar la petición"
    } else if e.is_body() || e.is_decode() {
        "fallo al leer/decodificar la respuesta de Google"
    } else {
        "error de red"
    };
    anyhow!("{context}: {kind}. (causa técnica: {e})")
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

/// (v3.38.0 #12) Recorta el cuerpo de error de Google a algo legible.
fn truncate_body(body: &str) -> String {
    let trimmed = body.trim();
    const MAX: usize = 300;
    if trimmed.chars().count() > MAX {
        let head: String = trimmed.chars().take(MAX).collect();
        format!("{head}… (recortado)")
    } else {
        trimmed.to_string()
    }
}

async fn find_sync_file(
    http: &reqwest::Client,
    access_token: &str,
) -> anyhow::Result<Option<String>> {
    let q = format!("name = '{}' and 'appDataFolder' in parents", SYNC_FILE_NAME);
    let url = format!(
        "https://www.googleapis.com/drive/v3/files?spaces=appDataFolder&q={}&fields=files(id)",
        urlencoding::encode(&q)
    );
    let resp = http
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| transport_error("Buscando el snapshot en Drive", e))?;
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
        "name": SYNC_FILE_NAME,
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
    // (v4.14.0 #2) Cuerpo como stream contador para reportar progreso de
    // subida. Mantenemos Content-Length fijo (= tamaño total) para que la
    // request NO use transfer-encoding chunked y el formato hacia Google
    // sea idéntico al buffered anterior.
    let total = body.len() as u64;
    let drive_body = counting_upload_body(body, total, app.cloned());
    let resp = http
        .post("https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart")
        .bearer_auth(access_token)
        .header(
            "Content-Type",
            format!("multipart/related; boundary={}", boundary),
        )
        .header(reqwest::header::CONTENT_LENGTH, total)
        .timeout(DRIVE_TRANSFER_TIMEOUT)
        .body(drive_body)
        .send()
        .await
        .map_err(|e| transport_error("Creando el snapshot en Drive", e))?;
    emit_transfer_progress(app, "upload", total, total, true);
    if !resp.status().is_success() {
        return Err(drive_error_for_response(resp).await);
    }
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
    // (v4.14.0 #2) Cuerpo como stream contador para reportar progreso.
    // Content-Length fijo = mismo formato de red que el buffered.
    let total = gz.len() as u64;
    let drive_body = counting_upload_body(gz, total, app.cloned());
    let resp = http
        .patch(format!(
            "https://www.googleapis.com/upload/drive/v3/files/{}?uploadType=media",
            file_id
        ))
        .bearer_auth(access_token)
        .header("Content-Type", "application/gzip")
        .header(reqwest::header::CONTENT_LENGTH, total)
        .timeout(DRIVE_TRANSFER_TIMEOUT)
        .body(drive_body)
        .send()
        .await
        .map_err(|e| transport_error("Actualizando el snapshot en Drive", e))?;
    emit_transfer_progress(app, "upload", total, total, true);
    if !resp.status().is_success() {
        return Err(drive_error_for_response(resp).await);
    }
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
) -> anyhow::Result<Option<String>> {
    let Some(file_id) = find_sync_file(http, access_token).await? else {
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
        .map_err(|e| transport_error("Descargando el snapshot de Drive", e))?;
    if !resp.status().is_success() {
        return Err(drive_error_for_response(resp).await);
    }
    // (v4.14.0 #2) Leemos la respuesta por streaming para reportar progreso
    // de bajada. `content_length` da el total cuando Drive lo envía (lo hace
    // para `alt=media`); si no, total=0 (barra indeterminada en el frontend).
    let total = resp.content_length().unwrap_or(0);
    let mut stream = resp.bytes_stream();
    let mut bytes: Vec<u8> = Vec::with_capacity(total as usize);
    let mut recv: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| transport_error("Descargando el snapshot de Drive", e))?;
        recv += chunk.len() as u64;
        bytes.extend_from_slice(&chunk);
        emit_transfer_progress(app, "download", recv, total, false);
    }
    emit_transfer_progress(app, "download", recv, total.max(recv), true);
    // (v4.0.1) El snapshot puede venir gzipeado (nuevo) o en texto plano
    // (snapshots viejos). `maybe_gunzip_to_string` detecta por magic bytes.
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
    if let Some(text) = download_sync_file(http, access_token, None).await? {
        if let Ok(snap) = serde_json::from_str::<Snapshot>(&text) {
            report.deleted_flights = snap.flight_log.len();
            report.deleted_tracks = snap.flight_log_track.len();
            report.deleted_settings = snap.settings.len();
        }
    }
    // Borrar el archivo de Drive si existe.
    if let Some(file_id) = find_sync_file(http, access_token).await? {
        let resp = http
            .delete(format!(
                "https://www.googleapis.com/drive/v3/files/{}",
                file_id
            ))
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| transport_error("Borrando el snapshot de Drive", e))?;
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
    sqlx::query("DELETE FROM app_settings WHERE key = ?1")
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
        version: 2,
        exported_at: chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string(),
        flight_log: flight_rows.iter().map(row_to_json).collect(),
        flight_log_track: track_rows.iter().map(row_to_json).collect(),
        settings: settings_rows,
        flight_log_score_item: score_rows.iter().map(row_to_json).collect(),
        flight_log_phase: phase_rows.iter().map(row_to_json).collect(),
    })
}

/// Convierte una fila genérica de SQLite a un `serde_json::Value`
/// objeto, con claves en camelCase y valores tipados según el `type
/// info` de SQLite (INTEGER → number, REAL → number, TEXT → string,
/// NULL → null).
fn row_to_json(row: &sqlx::sqlite::SqliteRow) -> serde_json::Value {
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

async fn restore_snapshot(pool: &SqlitePool, snap: &Snapshot) -> anyhow::Result<RestoreReport> {
    let mut tx = pool.begin().await?;
    let mut flights = 0;
    let mut tracks = 0;
    let mut settings = 0;
    for row in &snap.flight_log {
        let id = match row.get("id").and_then(|v| v.as_i64()) {
            Some(v) => v,
            None => continue,
        };
        let started_at = json_str(row, "startedAt").unwrap_or_default();
        // (v3.6.0 Phase H) Extendido para incluir flight_number,
        // callsign, airline_icao, status y los score_* del scoring
        // engine. Los campos del scoring SE sincronizan al cloud porque
        // el usuario los puede consultar desde cualquier máquina sin
        // tener que recomputar (decisión Q15 del kickoff).
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
                 score_total, score_max, score_grade)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
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
                 score_grade = excluded.score_grade"#,
        )
        .bind(id)
        .bind(&started_at)
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
        .bind(json_str(row, "flightNumber"))
        .bind(json_str(row, "callsign"))
        .bind(json_str(row, "airlineIcao"))
        .bind(json_str(row, "status").unwrap_or_else(|| "completed".to_string()))
        .bind(json_i64(row, "scoreTotal"))
        .bind(json_i64(row, "scoreMax"))
        .bind(json_str(row, "scoreGrade"))
        .execute(&mut *tx)
        .await?;
        flights += 1;
    }
    for row in &snap.flight_log_track {
        let flight_id = json_i64(row, "flightId").unwrap_or(0);
        let ts = json_str(row, "ts").unwrap_or_default();
        if flight_id == 0 || ts.is_empty() {
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
        let Some(flight_id) = json_i64(row, "flightId") else { continue };
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
        let Some(flight_id) = json_i64(row, "flightId") else { continue };
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

    Ok(RestoreReport {
        flights,
        tracks,
        settings,
    })
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
                name: "Credenciales locales".to_string(),
                ok: false,
                detail: format!("Error consultando credenciales: {e}"),
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
            name: "Credenciales locales".to_string(),
            ok: false,
            detail: "No hay Client ID / Secret embedidos. El binario fue compilado sin `secrets.local.toml` (gitignored) — re-buildea con el archivo presente en `src-tauri/`.".to_string(),
        });
        return CloudTestReport {
            overall_ok: false,
            steps,
            hint: Some(
                "Las credenciales OAuth vienen embedidas en el binario via build.rs. Coloca `src-tauri/secrets.local.toml` y recompila.".to_string(),
            ),
        };
    }
    let source = if HARDCODED_CLIENT_ID
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        "embedidas en el binario"
    } else {
        "leídas de DB (legacy v2.x)"
    };
    steps.push(CloudTestStep {
        name: "Credenciales locales".to_string(),
        ok: true,
        detail: format!(
            "Client ID + Secret OK ({} chars) — {}.",
            client_id.as_ref().map(|s| s.len()).unwrap_or(0),
            source
        ),
    });

    // Paso 2 — Refresh token
    let refresh_token = get_setting(pool, KEY_REFRESH_TOKEN).await.ok().flatten();
    if refresh_token.is_none() {
        steps.push(CloudTestStep {
            name: "Refresh token".to_string(),
            ok: false,
            detail: "Aún no completaste el OAuth. Pulsa 'Conectar con Google'.".to_string(),
        });
        return CloudTestReport {
            overall_ok: false,
            steps,
            hint: Some("Falta completar el flow OAuth.".to_string()),
        };
    }
    steps.push(CloudTestStep {
        name: "Refresh token".to_string(),
        ok: true,
        detail: "Tienes refresh_token guardado.".to_string(),
    });

    // Paso 3 — Access token (refresh)
    let access_token = match fresh_access_token(pool, http).await {
        Ok(t) => {
            steps.push(CloudTestStep {
                name: "Refresh access token".to_string(),
                ok: true,
                detail: format!("Access token nuevo obtenido ({} chars).", t.len()),
            });
            t
        }
        Err(e) => {
            steps.push(CloudTestStep {
                name: "Refresh access token".to_string(),
                ok: false,
                detail: format!("{}", e),
            });
            hint = Some(
                "El refresh_token fue rechazado. Probablemente: (a) revocaste el acceso de la app desde myaccount.google.com/permissions, o (b) la consent screen está en modo Testing y tu email no está en Test Users. Pulsa Desconectar + Conectar de nuevo.".to_string(),
            );
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
                .unwrap_or_else(|_| "(sin email)".to_string());
            steps.push(CloudTestStep {
                name: "userinfo (identidad)".to_string(),
                ok: true,
                detail: format!("Conectado como {}.", email),
            });
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            steps.push(CloudTestStep {
                name: "userinfo (identidad)".to_string(),
                ok: false,
                detail: format!("{}: {}", status, body),
            });
            hint = Some(
                "userinfo falló — el access_token no tiene la scope correcta. Desconecta y vuelve a Conectar.".to_string(),
            );
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint,
            };
        }
        Err(e) => {
            steps.push(CloudTestStep {
                name: "userinfo (identidad)".to_string(),
                ok: false,
                detail: format!("Error de red: {}", e),
            });
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint: Some("No hay conexión a internet o Google está caído.".to_string()),
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
                name: "Drive API alcanzable".to_string(),
                ok: true,
                detail: "GET /drive/v3/about respondió OK — la Drive API está activada y la scope es correcta.".to_string(),
            });
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            steps.push(CloudTestStep {
                name: "Drive API alcanzable".to_string(),
                ok: false,
                detail: format!("{}: {}", status, body),
            });
            if status.as_u16() == 403 {
                hint = Some(
                    "403 Forbidden en /drive/v3/about. Verifica que la Google Drive API esté activada EN EL MISMO proyecto donde creaste el OAuth client. Abre https://console.cloud.google.com/apis/library/drive.googleapis.com — el desplegable de arriba debe mostrar el proyecto de tu OAuth client.".to_string(),
                );
            }
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint,
            };
        }
        Err(e) => {
            steps.push(CloudTestStep {
                name: "Drive API alcanzable".to_string(),
                ok: false,
                detail: format!("Error de red: {}", e),
            });
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint: Some("Error de red.".to_string()),
            };
        }
    }

    // Paso 6 — listar appDataFolder (lo que sync_now hace primero)
    let q = format!("name = '{}' and 'appDataFolder' in parents", SYNC_FILE_NAME);
    let url = format!(
        "https://www.googleapis.com/drive/v3/files?spaces=appDataFolder&q={}&fields=files(id)",
        urlencoding::encode(&q)
    );
    match http.get(&url).bearer_auth(&access_token).send().await {
        Ok(resp) if resp.status().is_success() => {
            let list: DriveList = resp.json().await.unwrap_or(DriveList { files: vec![] });
            let detail = if list.files.is_empty() {
                "Listado vacío (aún no hay backup en Drive — es normal en primera vez).".to_string()
            } else {
                format!(
                    "Encontrado {} archivo(s) previo(s) en appDataFolder.",
                    list.files.len()
                )
            };
            steps.push(CloudTestStep {
                name: "Listar appDataFolder".to_string(),
                ok: true,
                detail,
            });
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            steps.push(CloudTestStep {
                name: "Listar appDataFolder".to_string(),
                ok: false,
                detail: format!("{}: {}", status, body),
            });
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint: Some("La scope `drive.appdata` no parece estar concedida. Desconecta y reconecta, asegúrate de aprobar TODAS las scopes en la consent screen.".to_string()),
            };
        }
        Err(e) => {
            steps.push(CloudTestStep {
                name: "Listar appDataFolder".to_string(),
                ok: false,
                detail: format!("Error de red: {}", e),
            });
            return CloudTestReport {
                overall_ok: false,
                steps,
                hint: Some("Error de red.".to_string()),
            };
        }
    }

    CloudTestReport {
        overall_ok: true,
        steps,
        hint: Some(
            "Todo OK — pulsa 'Sync ahora' para subir tu primera copia."
                .to_string(),
        ),
    }
}

pub async fn sync_now(
    pool: &SqlitePool,
    http: &reqwest::Client,
) -> anyhow::Result<SyncReport> {
    let access_token = fresh_access_token(pool, http).await?;
    let mut report = SyncReport::default();

    if let Some(text) = download_sync_file(http, &access_token, None).await? {
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

    let local = build_snapshot(pool).await?;
    let payload = serde_json::to_string(&local)?;
    report.uploaded_flights = local.flight_log.len();
    report.uploaded_tracks = local.flight_log_track.len();
    report.uploaded_settings = local.settings.len();

    if let Some(file_id) = find_sync_file(http, &access_token).await? {
        update_sync_file(http, &access_token, &file_id, &payload, None).await?;
    } else {
        create_sync_file(http, &access_token, &payload, None).await?;
    }

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
    tracing::info!(target: "cloud", "upload_all: construyendo snapshot local…");
    let access_token = fresh_access_token(pool, http).await?;
    let local = build_snapshot(pool).await?;
    let payload = serde_json::to_string(&local)?;
    let report = UploadReport {
        uploaded_flights: local.flight_log.len(),
        uploaded_tracks: local.flight_log_track.len(),
        uploaded_settings: local.settings.len(),
    };
    tracing::info!(
        target: "cloud",
        "upload_all: snapshot listo — {} vuelos, {} tracks, {} settings · JSON {:.1} MB (se sube gzipeado)",
        report.uploaded_flights,
        report.uploaded_tracks,
        report.uploaded_settings,
        payload.len() as f64 / 1_048_576.0,
    );
    if let Some(file_id) = find_sync_file(http, &access_token).await? {
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
        "upload OK · {} flights, {} tracks, {} settings",
        report.uploaded_flights, report.uploaded_tracks, report.uploaded_settings,
    );
    Ok(report)
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
    let access_token = fresh_access_token(pool, http).await?;
    let mut report = DownloadReport::default();

    let Some(text) = download_sync_file(http, &access_token, app).await? else {
        tracing::info!(target: "cloud", "download: no remote snapshot exists yet");
        return Ok(report);
    };
    let remote: Snapshot = match serde_json::from_str(&text) {
        Ok(s) => s,
        Err(e) => {
            return Err(anyhow!("snapshot remoto corrupto: {e:#}"));
        }
    };
    report.cloud_flights = remote.flight_log.len();

    // Construye índice de claves estables existentes en local para
    // hacer dedup eficiente. Un vuelo coincide si:
    //  - (source, external_id) matchea, o
    //  - (started_at, origin_lat round 2) matchea (sim-flown sin
    //    external_id)
    let local_keys = build_local_dedup_keys(pool).await?;

    // Mapa de cloud_flight_id → new_local_id (para tracks).
    let mut cloud_to_local_id: std::collections::HashMap<i64, i64> =
        std::collections::HashMap::new();

    let mut tx = pool.begin().await?;

    for row in &remote.flight_log {
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

        // (v3.6.0 Phase H) Incluye flight_number/callsign/airline_icao
        // /status/score_* en el INSERT para que un download-missing
        // preserve toda la metadata VA sin tener que recomputar.
        let result = sqlx::query(
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
                 score_total, score_max, score_grade)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"#,
        )
        .bind(&started_at)
        .bind(ended_at.as_deref())
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
        .bind(&source)
        .bind(external_id.as_deref())
        .bind(json_str(row, "flightNumber"))
        .bind(json_str(row, "callsign"))
        .bind(json_str(row, "airlineIcao"))
        .bind(json_str(row, "status").unwrap_or_else(|| "completed".to_string()))
        .bind(json_i64(row, "scoreTotal"))
        .bind(json_i64(row, "scoreMax"))
        .bind(json_str(row, "scoreGrade"))
        .execute(&mut *tx)
        .await?;

        let new_id = result.last_insert_rowid();
        if let Some(cid) = cloud_id {
            cloud_to_local_id.insert(cid, new_id);
        }
        report.new_flights += 1;
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
        .map(|p| std::path::Path::new(p).join(SYNC_FILE_NAME).is_file())
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
        .with_context(|| format!("No se pudo crear/abrir {}", path.display()))?;
    let out = path.join(SYNC_FILE_NAME);
    let bytes_written = payload.len() as u64;
    tokio::fs::write(&out, payload)
        .await
        .with_context(|| format!("No se pudo escribir en {}", out.display()))?;
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
    let path = std::path::PathBuf::from(folder_path).join(SYNC_FILE_NAME);
    if !path.is_file() {
        return Err(anyhow!(
            "No existe {} — no hay backup en esta carpeta",
            path.display()
        ));
    }
    let text = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("No se pudo leer {}", path.display()))?;
    let bytes_read = text.len() as u64;
    let snap: Snapshot = serde_json::from_str(&text)
        .with_context(|| "Archivo corrupto o de una versión incompatible")?;
    let restored = restore_snapshot(pool, &snap).await?;
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
