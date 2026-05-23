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

const SYNC_FILE_NAME: &str = "msfs-addons-data.json";
const SCOPE: &str = "https://www.googleapis.com/auth/drive.appdata https://www.googleapis.com/auth/userinfo.email";

// (v3.1.0) Credenciales embebidas en build — sólo dos usuarios (el
// owner y un amigo). Si los valores quedan vacíos, la app cae al
// flujo antiguo (settings.google_client_id/secret en DB) para devs.
// En producción se compilan con valores reales pasados por env var:
//   SIMFLEET_GOOGLE_CLIENT_ID=...
//   SIMFLEET_GOOGLE_CLIENT_SECRET=...
//
// `option_env!` los inserta como string slice si están presentes en
// build env; si no, devuelve None y caemos al fallback DB.
const HARDCODED_CLIENT_ID: Option<&str> =
    option_env!("SIMFLEET_GOOGLE_CLIENT_ID");
const HARDCODED_CLIENT_SECRET: Option<&str> =
    option_env!("SIMFLEET_GOOGLE_CLIENT_SECRET");

/// (v3.1.0) Lista blanca de emails Gmail autorizados a hacer sync.
/// La app rechaza el OAuth si el email del usuario no está aquí.
/// Hardcodeado por diseño: ESTE NO ES SOFTWARE PÚBLICO — son dos
/// usuarios. Cambiar la lista requiere recompilar.
const WHITELIST_EMAILS: &[&str] = &[
    // TODO: el owner debe rellenar con su email + el del amigo
    // antes de hacer el release. Si la lista queda vacía, sólo se
    // bloquea el sync cuando NO matchea nada (vacío = nada permitido).
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
    let client_id = get_setting(pool, KEY_CLIENT_ID)
        .await?
        .ok_or_else(|| anyhow!("Client ID no configurado"))?;
    let client_secret = get_setting(pool, KEY_CLIENT_SECRET)
        .await?
        .ok_or_else(|| anyhow!("Client Secret no configurado"))?;
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

/// (v2.0.1) Convierte un error HTTP de Drive en un mensaje accionable.
/// 403 típicamente significa "API no activada en tu proyecto" — que
/// es un paso oculto al crear el OAuth client. Devolvemos un mensaje
/// con la URL exacta para activar.
async fn drive_error_for_response(resp: reqwest::Response) -> anyhow::Error {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if status.as_u16() == 403 {
        return anyhow!(
            "Google Drive API 403 Forbidden. Probablemente no activaste la \
             Google Drive API en tu proyecto de Google Cloud. Abre \
             https://console.cloud.google.com/apis/library/drive.googleapis.com, \
             selecciona TU proyecto (arriba), y pulsa 'Enable'. Detalle: {}",
            body
        );
    }
    if status.as_u16() == 401 {
        return anyhow!(
            "Google Drive API 401 Unauthorized. El access_token expiró o las \
             credenciales son inválidas. Pulsa Desconectar y vuelve a \
             Conectar. Detalle: {}",
            body
        );
    }
    anyhow!("Google Drive {} : {}", status, body)
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
        .await?;
    if !resp.status().is_success() {
        return Err(drive_error_for_response(resp).await);
    }
    let list: DriveList = resp.json().await?;
    Ok(list.files.into_iter().next().map(|f| f.id))
}

async fn create_sync_file(
    http: &reqwest::Client,
    access_token: &str,
    content: &str,
) -> anyhow::Result<String> {
    let metadata = serde_json::json!({
        "name": SYNC_FILE_NAME,
        "parents": ["appDataFolder"],
    });
    let boundary = "msfs_addons_boundary_42";
    let body = format!(
        "--{b}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{b}\r\nContent-Type: application/json\r\n\r\n{content}\r\n--{b}--",
        b = boundary,
        meta = metadata,
        content = content,
    );
    let resp = http
        .post("https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart")
        .bearer_auth(access_token)
        .header(
            "Content-Type",
            format!("multipart/related; boundary={}", boundary),
        )
        .body(body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(drive_error_for_response(resp).await);
    }
    let file: DriveFile = resp.json().await?;
    Ok(file.id)
}

async fn update_sync_file(
    http: &reqwest::Client,
    access_token: &str,
    file_id: &str,
    content: &str,
) -> anyhow::Result<()> {
    let resp = http
        .patch(format!(
            "https://www.googleapis.com/upload/drive/v3/files/{}?uploadType=media",
            file_id
        ))
        .bearer_auth(access_token)
        .header("Content-Type", "application/json")
        .body(content.to_string())
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(drive_error_for_response(resp).await);
    }
    Ok(())
}

async fn download_sync_file(
    http: &reqwest::Client,
    access_token: &str,
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
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(drive_error_for_response(resp).await);
    }
    Ok(Some(resp.text().await?))
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
    let settings_rows: Vec<SettingKV> = sqlx::query_as(
        "SELECT key, value FROM settings WHERE key LIKE 'pref_%'",
    )
    .fetch_all(pool)
    .await?;
    Ok(Snapshot {
        version: 1,
        exported_at: chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string(),
        flight_log: flight_rows.iter().map(row_to_json).collect(),
        flight_log_track: track_rows.iter().map(row_to_json).collect(),
        settings: settings_rows,
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
        sqlx::query(
            r#"INSERT INTO flight_log
                (id, started_at, ended_at, origin_icao, origin_name, origin_lat, origin_lon,
                 destination_icao, destination_name, destination_lat, destination_lon,
                 aircraft_atc_type, aircraft_title, distance_nm, flight_time_s,
                 max_altitude_ft, landing_fpm, max_ground_speed_kt, max_true_airspeed_kt,
                 departure_gate, arrival_gate, passengers, cargo_kg, fuel_used_kg,
                 paused_seconds)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
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
                 paused_seconds = excluded.paused_seconds"#,
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
        sqlx::query(
            // (v2.2.0) FIX: la columna real en la tabla es `gs_kt`, no
            // `ground_speed_kt`. El error `(code: 1) table flight_log_track
            // has no column named ground_speed_kt` que reportó el usuario
            // venía de aquí. El JSON de la snapshot puede contener cualquiera
            // de las dos keys según la versión que la generó, así que el
            // restore acepta ambas via groundSpeedKt / gsKt.
            "INSERT OR IGNORE INTO flight_log_track (flight_id, ts, lat, lon, alt_ft, gs_kt) VALUES (?,?,?,?,?,?)",
        )
        .bind(flight_id)
        .bind(&ts)
        .bind(json_f64(row, "lat").unwrap_or(0.0))
        .bind(json_f64(row, "lon").unwrap_or(0.0))
        .bind(json_i64(row, "altFt").unwrap_or(0))
        .bind(
            // (v2.2.0) Compat: snapshots viejas usan "groundSpeedKt",
            // las nuevas (dynamic row_to_json) usan "gsKt". Aceptar
            // ambas.
            json_i64(row, "gsKt")
                .or_else(|| json_i64(row, "groundSpeedKt"))
                .unwrap_or(0),
        )
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
    tx.commit().await?;
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

    // Paso 1 — Credenciales locales
    let client_id = get_setting(pool, KEY_CLIENT_ID).await.ok().flatten();
    let client_secret = get_setting(pool, KEY_CLIENT_SECRET).await.ok().flatten();
    if client_id.is_none() || client_secret.is_none() {
        steps.push(CloudTestStep {
            name: "Credenciales locales".to_string(),
            ok: false,
            detail: "Falta Client ID y/o Client Secret. Pulsa 'Configurar credenciales' arriba.".to_string(),
        });
        return CloudTestReport {
            overall_ok: false,
            steps,
            hint: Some("Configura primero el Client ID + Client Secret de un OAuth client tipo 'Desktop app'.".to_string()),
        };
    }
    steps.push(CloudTestStep {
        name: "Credenciales locales".to_string(),
        ok: true,
        detail: format!(
            "Client ID guardado ({} chars), Client Secret guardado.",
            client_id.as_ref().unwrap().len()
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

    if let Some(text) = download_sync_file(http, &access_token).await? {
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
        update_sync_file(http, &access_token, &file_id, &payload).await?;
    } else {
        create_sync_file(http, &access_token, &payload).await?;
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
