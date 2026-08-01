//! (v7.1) i18n de BACKEND.
//!
//! El backend no puede llamar al `t()` del frontend, así que los mensajes
//! user-facing que devuelve (errores de comando, `DownloadJob.message/error`,
//! payloads de eventos) salían SIEMPRE en español → un usuario con la app en
//! inglés veía español. Esta capa resuelve eso: un locale cacheado en memoria
//! + un diccionario `(key, es, en)`.
//!
//! El locale lo empuja el FRONTEND (que ya resuelve "auto" vía navigator) con
//! el comando `set_backend_locale` en el bootstrap y al cambiar idioma. Hasta
//! que llega, el default es Español — el mismo comportamiento de siempre.
//!
//! Uso:
//! ```ignore
//! anyhow::bail!("{}", crate::tr!("install.no_file", path = path.display()));
//! let msg = crate::tr!("dl.mgr_unavailable"); // sin args
//! ```

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Es,
    En,
}

static LOCALE: RwLock<Lang> = RwLock::new(Lang::Es);

/// Fija el locale del backend. `code` viene del frontend ("es"/"en"); cualquier
/// otra cosa cae a Español.
pub fn set_locale(code: &str) {
    let lang = if code.eq_ignore_ascii_case("en") {
        Lang::En
    } else {
        Lang::Es
    };
    if let Ok(mut w) = LOCALE.write() {
        *w = lang;
    }
}

pub fn current() -> Lang {
    LOCALE.read().map(|g| *g).unwrap_or(Lang::Es)
}

/// Traduce `key` al locale activo. Devuelve la `key` cruda si no está en el
/// diccionario (así una clave rota se nota en vez de crashear).
pub fn tr(key: &str) -> String {
    match dict().get(key) {
        Some((es, en)) => match current() {
            Lang::Es => es,
            Lang::En => en,
        }
        .to_string(),
        None => key.to_string(),
    }
}

fn dict() -> &'static HashMap<&'static str, (&'static str, &'static str)> {
    static M: OnceLock<HashMap<&'static str, (&'static str, &'static str)>> = OnceLock::new();
    M.get_or_init(|| ENTRIES.iter().map(|(k, es, en)| (*k, (*es, *en))).collect())
}

/// Macro de traducción con interpolación por nombre. `crate::tr!("k")` o
/// `crate::tr!("k", nombre = valor)` → reemplaza `{nombre}` en la plantilla.
#[macro_export]
macro_rules! tr {
    ($key:expr) => { $crate::i18n::tr($key) };
    ($key:expr, $($n:ident = $v:expr),+ $(,)?) => {{
        let mut s = $crate::i18n::tr($key);
        $( s = s.replace(concat!("{", stringify!($n), "}"), &format!("{}", $v)); )+
        s
    }};
}

// (key, es, en). Las plantillas usan `{nombre}` para los args de `tr!`.
// Poblado por la limpieza de i18n de backend (v7.1).
#[rustfmt::skip]
static ENTRIES: &[(&str, &str, &str)] = &[
    ("backup.communityNotDetected", "No se detectó la carpeta Community.", "Community folder not detected."),
    ("backup.invalidBase64", "base64 inválido: {e}", "invalid base64: {e}"),
    ("cloudSync.accessTokenObtained", "Access token nuevo obtenido ({chars} chars).", "New access token obtained ({chars} chars)."),
    ("cloudSync.appDataFolderEmpty", "Listado vacío (aún no hay backup en Drive — es normal en primera vez).", "Empty listing (no backup in Drive yet — normal the first time)."),
    ("cloudSync.appDataFolderFound", "Encontrado {n} archivo(s) previo(s) en appDataFolder.", "Found {n} previous file(s) in appDataFolder."),
    ("cloudSync.authTimeout", "Timeout esperando autorización (5 min)", "Timed out waiting for authorization (5 min)"),
    ("cloudSync.callbackInvalid", "Callback inválido (state o code faltantes)", "Invalid callback (missing state or code)"),
    ("cloudSync.clientIdNotEmbedded", "Client ID no embebido en el binario (re-buildea con secrets.local.toml)", "Client ID not embedded in the binary (rebuild with secrets.local.toml)"),
    ("cloudSync.clientSecretNotEmbedded", "Client Secret no embebido en el binario", "Client Secret not embedded in the binary"),
    ("cloudSync.connectedAsDetail", "Conectado como {email}.", "Connected as {email}."),
    ("cloudSync.credsNotEmbedded", "Las credenciales de Google no están embebidas en esta build. Si ves este mensaje en producción, contacta al desarrollador — la app debió compilarse con SIMFLEET_GOOGLE_CLIENT_ID/SECRET.", "Google credentials are not embedded in this build. If you see this message in production, contact the developer — the app should have been compiled with SIMFLEET_GOOGLE_CLIENT_ID/SECRET."),
    ("cloudSync.credsNotEmbeddedShort", "Las credenciales de Google no están embebidas en esta build.", "Google credentials are not embedded in this build."),
    ("cloudSync.credsOkDetail", "Client ID + Secret OK ({chars} chars) — {source}.", "Client ID + Secret OK ({chars} chars) — {source}."),
    ("cloudSync.credsQueryError", "Error consultando credenciales: {e}", "Error querying credentials: {e}"),
    ("cloudSync.credsSourceDb", "leídas de DB (legacy v2.x)", "read from the DB (legacy v2.x)"),
    ("cloudSync.credsSourceEmbedded", "embedidas en el binario", "embedded in the binary"),
    ("cloudSync.ctxCreatingSnapshot", "Creando el snapshot en Drive", "Creating the snapshot in Drive"),
    ("cloudSync.ctxDeletingSnapshot", "Borrando el snapshot de Drive", "Deleting the snapshot from Drive"),
    ("cloudSync.ctxDownloadingSnapshot", "Descargando el snapshot de Drive", "Downloading the snapshot from Drive"),
    ("cloudSync.ctxSearchingSnapshot", "Buscando el snapshot en Drive", "Searching for the snapshot in Drive"),
    ("cloudSync.ctxUpdatingSnapshot", "Actualizando el snapshot en Drive", "Updating the snapshot in Drive"),
    ("cloudSync.drive401", "Google Drive 401 (no autorizado). El access_token expiró o las credenciales son inválidas. Pulsa Desconectar y vuelve a Conectar. Detalle: {body}", "Google Drive 401 (unauthorized). The access_token expired or the credentials are invalid. Press Disconnect and Connect again. Detail: {body}"),
    ("cloudSync.drive403", "Google Drive 403 (prohibido). Lo más común: no activaste la Google Drive API en tu proyecto de Google Cloud. Ábrela en https://console.cloud.google.com/apis/library/drive.googleapis.com (selecciona TU proyecto, arriba) y pulsa 'Enable'. También puede ser cuota del proyecto excedida. Detalle: {body}", "Google Drive 403 (forbidden). Most commonly: you didn't enable the Google Drive API in your Google Cloud project. Open it at https://console.cloud.google.com/apis/library/drive.googleapis.com (select YOUR project, at the top) and press 'Enable'. It could also be an exceeded project quota. Detail: {body}"),
    ("cloudSync.drive404", "Google Drive 404 (no encontrado). El archivo de sync remoto ya no existe (se borró o se purgó). Reintenta: se recreará solo. Detalle: {body}", "Google Drive 404 (not found). The remote sync file no longer exists (it was deleted or purged). Retry: it will recreate itself. Detail: {body}"),
    ("cloudSync.drive429", "Google Drive 429 (demasiadas peticiones). Rate-limit/cuota excedido. Espera un momento y reintenta. Detalle: {body}", "Google Drive 429 (too many requests). Rate limit/quota exceeded. Wait a moment and retry. Detail: {body}"),
    ("cloudSync.drive5xx", "Google Drive {code} (error del servidor de Google — no es tu configuración). Reintenta en unos minutos. Detalle: {body}", "Google Drive {code} (Google server error — not your configuration). Retry in a few minutes. Detail: {body}"),
    ("cloudSync.driveAboutOk", "GET /drive/v3/about respondió OK — la Drive API está activada y la scope es correcta.", "GET /drive/v3/about responded OK — the Drive API is enabled and the scope is correct."),
    ("cloudSync.emailFetchFailed", "No se pudo obtener el email del usuario para validar la whitelist.", "Could not obtain the user's email to validate the whitelist."),
    ("cloudSync.emailNotAuthorized", "El email '{email}' no está autorizado para sincronizar. Contacta al desarrollador para añadirlo a la whitelist.", "The email '{email}' is not authorized to sync. Contact the developer to add it to the whitelist."),
    ("cloudSync.fileCorruptOrIncompatible", "Archivo corrupto o de una versión incompatible", "Corrupt file or from an incompatible version"),
    ("cloudSync.folderBackupMissing", "No existe {path} — no hay backup en esta carpeta", "{path} does not exist — there is no backup in this folder"),
    ("cloudSync.folderCreateFailed", "No se pudo crear/abrir {path}", "Could not create/open {path}"),
    ("cloudSync.folderLoadWouldOverwrite", "Ya tenés {n} vuelos en esta PC. Cargar el backup por carpeta los sobreescribiría por id. Usá la sincronización de nube (botón «Bajar»), que mergea por claves sin pisar nada.", "You already have {n} flights on this PC. Loading the folder backup would overwrite them by id. Use cloud sync (the «Download» button), which merges by keys without overwriting anything."),
    ("cloudSync.folderReadFailed", "No se pudo leer {path}", "Could not read {path}"),
    ("cloudSync.folderWriteFailed", "No se pudo escribir en {path}", "Could not write to {path}"),
    ("cloudSync.hintAllOk", "Todo OK — pulsa 'Sync ahora' para subir tu primera copia.", "All OK — press 'Sync now' to upload your first copy."),
    ("cloudSync.hintCompleteOauth", "Falta completar el flow OAuth.", "The OAuth flow still needs to be completed."),
    ("cloudSync.hintCredsEmbedded", "Las credenciales OAuth vienen embedidas en el binario via build.rs. Coloca `src-tauri/secrets.local.toml` y recompila.", "The OAuth credentials are embedded in the binary via build.rs. Place `src-tauri/secrets.local.toml` and recompile."),
    ("cloudSync.hintDrive403", "403 Forbidden en /drive/v3/about. Verifica que la Google Drive API esté activada EN EL MISMO proyecto donde creaste el OAuth client. Abre https://console.cloud.google.com/apis/library/drive.googleapis.com — el desplegable de arriba debe mostrar el proyecto de tu OAuth client.", "403 Forbidden on /drive/v3/about. Verify that the Google Drive API is enabled IN THE SAME project where you created the OAuth client. Open https://console.cloud.google.com/apis/library/drive.googleapis.com — the dropdown at the top must show your OAuth client's project."),
    ("cloudSync.hintNoInternet", "No hay conexión a internet o Google está caído.", "No internet connection or Google is down."),
    ("cloudSync.hintRefreshRejected", "El refresh_token fue rechazado. Probablemente: (a) revocaste el acceso de la app desde myaccount.google.com/permissions, o (b) la consent screen está en modo Testing y tu email no está en Test Users. Pulsa Desconectar + Conectar de nuevo.", "The refresh_token was rejected. Most likely: (a) you revoked the app's access from myaccount.google.com/permissions, or (b) the consent screen is in Testing mode and your email is not in Test Users. Press Disconnect + Connect again."),
    ("cloudSync.hintScopeNotGranted", "La scope `drive.appdata` no parece estar concedida. Desconecta y reconecta, asegúrate de aprobar TODAS las scopes en la consent screen.", "The `drive.appdata` scope doesn't appear to be granted. Disconnect and reconnect, and make sure to approve ALL scopes on the consent screen."),
    ("cloudSync.hintUserinfoFailed", "userinfo falló — el access_token no tiene la scope correcta. Desconecta y vuelve a Conectar.", "userinfo failed — the access_token doesn't have the correct scope. Disconnect and Connect again."),
    ("cloudSync.listenerOpenFailed", "No se pudo abrir el listener local para el callback OAuth", "Could not open the local listener for the OAuth callback"),
    ("cloudSync.networkErrorDetail", "Error de red: {e}", "Network error: {e}"),
    ("cloudSync.networkErrorHint", "Error de red.", "Network error."),
    ("cloudSync.noClientIdSecretEmbedded", "No hay Client ID / Secret embedidos. El binario fue compilado sin `secrets.local.toml` (gitignored) — re-buildea con el archivo presente en `src-tauri/`.", "No Client ID / Secret embedded. The binary was compiled without `secrets.local.toml` (gitignored) — rebuild with the file present in `src-tauri/`."),
    ("cloudSync.noEmailPlaceholder", "(sin email)", "(no email)"),
    ("cloudSync.noRefreshToken", "Google no devolvió refresh_token. Re-intenta con la app de OAuth en modo 'Desktop' y consent screen aprobado.", "Google did not return a refresh_token. Retry with the OAuth app in 'Desktop' mode and an approved consent screen."),
    ("cloudSync.notConnected", "No conectado — pulsa Conectar primero", "Not connected — press Connect first"),
    ("cloudSync.oauthNotCompletedDetail", "Aún no completaste el OAuth. Pulsa 'Conectar con Google'.", "You haven't completed OAuth yet. Press 'Connect with Google'."),
    ("cloudSync.oauthPageErrorBody", "Vuelve a la app e inténtalo de nuevo.", "Return to the app and try again."),
    ("cloudSync.oauthPageErrorTitle", "Error en la autorización", "Authorization error"),
    ("cloudSync.oauthPageOkBody", "Puedes cerrar esta pestaña y volver a la app.", "You can close this tab and return to the app."),
    ("cloudSync.oauthPageOkTitle", "Conectado correctamente", "Connected successfully"),
    ("cloudSync.refreshTokenRejected", "Refresh token rechazado ({status}): {body}. Vuelve a Conectar.", "Refresh token rejected ({status}): {body}. Please Connect again."),
    ("cloudSync.refreshTokenSaved", "Tienes refresh_token guardado.", "You have a saved refresh_token."),
    ("cloudSync.remoteSnapshotCorrupt", "snapshot remoto corrupto: {e}", "corrupt remote snapshot: {e}"),
    ("cloudSync.stepDriveReachable", "Drive API alcanzable", "Drive API reachable"),
    ("cloudSync.stepListAppDataFolder", "Listar appDataFolder", "List appDataFolder"),
    ("cloudSync.stepLocalCreds", "Credenciales locales", "Local credentials"),
    ("cloudSync.stepRefreshAccessToken", "Refresh access token", "Refresh access token"),
    ("cloudSync.stepRefreshToken", "Refresh token", "Refresh token"),
    ("cloudSync.stepUserinfo", "userinfo (identidad)", "userinfo (identity)"),
    ("cloudSync.tokenEndpointError", "Google /token devolvió {status}: {body}", "Google /token returned {status}: {body}"),
    ("cloudSync.transportConnect", "no se pudo conectar con Google (sin internet, fallo de DNS, o un firewall/proxy bloqueando *.googleapis.com)", "could not connect to Google (no internet, DNS failure, or a firewall/proxy blocking *.googleapis.com)"),
    ("cloudSync.transportDecode", "fallo al leer/decodificar la respuesta de Google", "failed to read/decode Google's response"),
    ("cloudSync.transportGeneric", "error de red", "network error"),
    ("cloudSync.transportRequest", "no se pudo construir/enviar la petición", "could not build/send the request"),
    ("cloudSync.transportTimeout", "timeout — Google tardó demasiado en responder (revisa tu internet, o un firewall/proxy/antivirus que esté bloqueando la conexión)", "timeout — Google took too long to respond (check your internet, or a firewall/proxy/antivirus blocking the connection)"),
    ("cloudSync.transportWrap", "{context}: {kind}. (causa técnica: {e})", "{context}: {kind}. (technical cause: {e})"),
    ("cloudSync.truncatedSuffix", "(recortado)", "(truncated)"),
    ("community.folderNotDetected", "No se detectó la carpeta Community automáticamente — pásala manualmente.", "The Community folder was not detected automatically — pass it manually."),
    ("community.folderNotExist", "La carpeta Community detectada no existe: {path}", "The detected Community folder does not exist: {path}"),
    ("community.linkSelf", "un addon no puede enlazarse consigo mismo", "an addon cannot be linked to itself"),
    ("community.pmdgScanTaskFailed", "la tarea de escaneo PMDG falló: {e}", "the PMDG scan task failed: {e}"),
    ("community.scanTaskFailed", "la tarea de escaneo falló: {e}", "the scan task failed: {e}"),
    ("community.unknownPackage", "paquete desconocido: {folder}", "unknown package: {folder}"),
    ("crossLink.badJunctionChars", "nombre de carpeta con caracteres no permitidos para junction: {path}", "folder name with characters not allowed for a junction: {path}"),
    ("crossLink.destCreateFailed", "{name}: no se pudo crear la Community destino ({e})", "{name}: could not create the destination Community ({e})"),
    ("crossLink.mklinkFailed", "mklink /J falló: {err} {out}", "mklink /J failed: {err} {out}"),
    ("crossLink.sourceMissing", "{name}: la carpeta de origen no existe", "{name}: the source folder does not exist"),
    ("db.diagIcaoNotInAirports", "El ICAO '{icao}' no existe en la tabla 'airports' (dataset OurAirports). Sin esto se descartan falsos positivos. Si el ICAO es real, dispara 'refresh airports dataset' o verifica que se hayan descargado los datos.", "The ICAO '{icao}' does not exist in the 'airports' table (OurAirports dataset). Without it, false positives are discarded. If the ICAO is real, trigger 'refresh airports dataset' or check that the data has been downloaded."),
    ("db.diagNoCatalogVersion", "El catálogo (addons) no tiene ninguna entrada con versión para ICAO '{icao}'. Causas: la búsqueda en SceneryAddons/Simplaza no devolvió nada con ese ICAO, o el parser no extrajo la versión del título. Pulsa 'Refresh updates' o busca el ICAO manualmente para alimentar la cache.", "The catalog (addons) has no versioned entry for ICAO '{icao}'. Causes: the SceneryAddons/Simplaza search returned nothing for that ICAO, or the parser did not extract the version from the title. Press 'Refresh updates' or search the ICAO manually to feed the cache."),
    ("db.diagNoIcao", "El paquete no tiene ICAO extraído. Sin ICAO no se puede cruzar con el catálogo. Causas típicas: el manifest no tiene 'SCENERY' como content_type, o el título/folder no contiene un código ICAO de 4 letras.", "The package has no extracted ICAO. Without an ICAO it cannot be matched against the catalog. Typical causes: the manifest does not have 'SCENERY' as content_type, or the title/folder does not contain a 4-letter ICAO code."),
    ("db.diagNoVersion", "El manifest no declara 'package_version'. Sin versión instalada no se puede comparar.", "The manifest does not declare 'package_version'. Without an installed version there is nothing to compare."),
    ("db.diagPackageMissing", "El paquete '{folder}' no está en community_packages — re-escanea Community.", "Package '{folder}' is not in community_packages — re-scan Community."),
    ("db.diagUpToDate", "Versión instalada '{installed}' >= mejor versión catalogada '{latest}' — no hay update real.", "Installed version '{installed}' >= best cataloged version '{latest}' — there is no real update."),
    ("db.diagWrongContentType", "content_type es '{contentType}' — la detección de updates exige 'SCENERY'. Si crees que es scenery, edita el manifest.", "content_type is '{contentType}' — update detection requires 'SCENERY'. If you think it is scenery, edit the manifest."),
    ("db.emptyValue", "(vacío)", "(empty)"),
    ("discord.atAirport", "En {airport}", "At {airport}"),
    ("discord.flying", "Volando", "Flying"),
    ("discord.idleDetails", "En SimFleet", "In SimFleet"),
    ("discord.idleState", "Gestionando addons", "Managing addons"),
    ("discord.phaseApproach", "En aproximación", "On approach"),
    ("discord.phaseClimbing", "En ascenso", "Climbing"),
    ("discord.phaseCruise", "En crucero", "Cruising"),
    ("discord.phaseDeboarding", "Desembarque", "Deboarding"),
    ("discord.phaseDescent", "En descenso", "Descending"),
    ("discord.phaseEngineRunning", "Motores en marcha", "Engines running"),
    ("discord.phaseInFlight", "En vuelo", "In flight"),
    ("discord.phaseLanding", "Aterrizando", "Landing"),
    ("discord.phaseParking", "En puerta", "At the gate"),
    ("discord.phasePreflight", "Prevuelo", "Preflight"),
    ("discord.phasePushback", "Pushback", "Pushback"),
    ("discord.phaseTakeoff", "Despegue", "Takeoff"),
    ("discord.phaseTaxiIn", "Rodaje a puerta", "Taxiing in"),
    ("discord.phaseTaxiOut", "Rodaje a pista", "Taxiing out"),
    ("drop.fallbackFile", "archivo", "file"),
    ("drop.fallbackPackage", "paquete", "package"),
    ("dropCmd.communityNotDetected", "Community folder no detectada", "Community folder not detected"),
    ("dropCmd.deleteFailed", "No se pudo borrar {path}: {e}", "Couldn't delete {path}: {e}"),
    ("dropCmd.unknown", "desconocido", "unknown"),
    ("dropInstall.appInstallerLabel", "App / instalador · {name}", "App / installer · {name}"),
    ("dropInstall.fileNoName", "archivo sin nombre", "file with no name"),
    ("dropInstall.folder", "carpeta: {name}", "folder: {name}"),
    ("dropInstall.folderNoContent", "La carpeta no contiene paquetes MSFS (manifest+layout) ni perfiles GSX.", "The folder doesn't contain MSFS packages (manifest+layout) or GSX profiles."),
    ("dropInstall.folderNoName", "carpeta sin nombre", "folder with no name"),
    ("dropInstall.gsxPathUnresolved", "No se pudo resolver %APPDATA%\\Virtuali\\GSX\\MSFS", "Couldn't resolve %APPDATA%\\Virtuali\\GSX\\MSFS"),
    ("dropInstall.itemNotInSession", "Item no encontrado en sesión: {path}", "Item not found in session: {path}"),
    ("dropInstall.language", "idioma: {code}", "language: {code}"),
    ("dropInstall.mutexPoisoned", "Mutex envenenado: {e}", "Poisoned mutex: {e}"),
    ("dropInstall.notForCommunity", "no va en Community", "not for Community"),
    ("dropInstall.notInstallableType", "Tipo no instalable: {kind}", "Non-installable type: {kind}"),
    ("dropInstall.package", "paquete", "package"),
    ("dropInstall.packageNotFound", "No encuentro ningún paquete {prefix}-* en ninguna carpeta de estado de MSFS. ¿Está instalado el avión?", "Couldn't find any {prefix}-* package in any MSFS state folder. Is the aircraft installed?"),
    ("dropInstall.pathNotExist", "No existe la ruta {path}", "Path does not exist: {path}"),
    ("dropInstall.sessionMissing", "Sesión {id} no existe o expiró", "Session {id} doesn't exist or expired"),
    ("dropInstall.tempdirFailed", "no se pudo crear tempdir: {e}", "couldn't create tempdir: {e}"),
    ("dropInstall.unknownConfigKind", "kind de config desconocido: {kind}", "unknown config kind: {kind}"),
    ("dropInstall.unsupportedExt", "Extensión no soportada: .{ext} (acepta .ini, .py, .zip, .rar, .7z, .exe, .msi)", "Unsupported extension: .{ext} (accepts .ini, .py, .zip, .rar, .7z, .exe, .msi)"),
    ("dropInstall.withVdgs", "con VDGS", "with VDGS"),
    ("dropInstall.withoutVdgs", "sin VDGS", "without VDGS"),
    ("economy.aircraftPurchase", "{route} · compra avión", "{route} · aircraft purchase"),
    ("economy.flight", "Vuelo", "Flight"),
    ("economy.maintenance", "Mantenimiento: {comp}", "Maintenance: {comp}"),
    ("economy.start", "Inicio", "Start"),
    ("gsx.copyError", "Error copiando {path}: {e}", "Error copying {path}: {e}"),
    ("gsx.fileNoName", "archivo sin nombre", "file without a name"),
    ("gsx.fileNotFound", "No existe el archivo {path}", "The file {path} does not exist"),
    ("gsx.fileTooLarge", "Archivo muy grande ({bytes} bytes); cap 10MB", "File too large ({bytes} bytes); cap 10MB"),
    ("gsx.noProfilesInArchive", "El archivo .{kind} no contiene perfiles GSX (.ini/.py)", "The .{kind} file contains no GSX profiles (.ini/.py)"),
    ("gsx.rarExtractError", "Error extrayendo .rar: {e}", "Error extracting .rar: {e}"),
    ("gsx.rarOpenError", "Error abriendo .rar: {e}", "Error opening .rar: {e}"),
    ("gsx.rarReadError", "Error leyendo .rar: {e}", "Error reading .rar: {e}"),
    ("gsx.resolveAppdataFailed", "No se pudo resolver %APPDATA%\\Virtuali\\GSX\\MSFS", "Could not resolve %APPDATA%\\Virtuali\\GSX\\MSFS"),
    ("gsx.unsupportedExt", "Extensión no soportada: .{ext} (acepta .ini, .py, .zip, .rar)", "Unsupported extension: .{ext} (accepts .ini, .py, .zip, .rar)"),
    ("gsx.zipExtractError", "Error extrayendo .zip: {e}", "Error extracting .zip: {e}"),
    ("install.communityMissing", "La carpeta Community no existe: {path}", "The Community folder does not exist: {path}"),
    ("install.copyFailed", "falló la copia: {from} → {to}", "copy failed: {from} → {to}"),
    ("install.extractFailed7z", "no se pudo extraer {path}", "could not extract {path}"),
    ("install.extractionFailed", "la extracción falló", "extraction failed"),
    ("install.fileNotFound", "No se encontró el archivo: {path}", "File not found: {path}"),
    ("install.noPackagesOrInstallers", "No se encontraron paquetes MSFS (manifest.json + layout.json) ni instaladores ejecutables dentro del archivo.", "No MSFS packages (manifest.json + layout.json) or executable installers were found inside the archive."),
    ("install.noParentDir", "el archivo no tiene un directorio padre", "the file has no parent directory"),
    ("install.nonUtf8PackageName", "el paquete tiene un nombre de carpeta no-UTF-8: {path}", "the package has a non-UTF-8 folder name: {path}"),
    ("install.openArchiveFailed", "no se pudo abrir el archivo {path}", "could not open the archive {path}"),
    ("install.openFailed7z", "no se pudo abrir {path}", "could not open {path}"),
    ("install.openRarFailed", "no se pudo abrir el archivo RAR {path}", "could not open the RAR archive {path}"),
    ("install.persistFailed", "no se pudo persistir el extracto en {path}", "could not persist the extracted files at {path}"),
    ("install.ptpUnsupported", "Los archivos .ptp (liveries PMDG) están encriptados y no son soportados por esta app. Instálalos directamente con PMDG Operations Center.", ".ptp files (PMDG liveries) are encrypted and are not supported by this app. Install them directly with PMDG Operations Center."),
    ("install.rarExtractFailed", "extracción RAR falló", "RAR extraction failed"),
    ("install.rarHeaderInvalid", "header RAR inválido", "invalid RAR header"),
    ("install.rarSkipFailed", "skip RAR falló", "RAR skip failed"),
    ("install.removePrevFailed", "no se pudo eliminar la instalación previa en {path} (¿archivo en uso?)", "could not remove the previous installation at {path} (file in use?)"),
    ("install.tempDirFailed", "no se pudo crear el directorio temporal de extracción", "could not create the temporary extraction directory"),
    ("install.unsupportedFormat", "Formato de archivo no soportado: .{ext} (se esperaba .zip, .rar o .7z)", "Unsupported archive format: .{ext} (expected .zip, .rar or .7z)"),
    ("install.zipReadFailed", "no se pudo leer el archivo ZIP", "could not read the ZIP archive"),
    ("installCmd.communityNotDetected", "No se detectó la carpeta Community automáticamente — configúrala desde Ajustes.", "The Community folder could not be detected automatically — set it in Settings."),
    ("installCmd.taskFailed", "la tarea de instalación falló: {e}", "the installation task failed: {e}"),
    ("livery.downloadFailed", "La descarga falló. Tocá Cancelar y reintentá desde SimFleet.", "The download failed. Tap Cancel and retry from SimFleet."),
    ("livery.downloadFailedConn", "La descarga falló (el hoster cortó la conexión). Tocá Cancelar y reintentá desde SimFleet.", "The download failed (the host dropped the connection). Tap Cancel and retry from SimFleet."),
    ("livery.downloadsFolderNotFound", "No pude localizar la carpeta de Descargas", "Couldn't locate the Downloads folder"),
    ("livery.fileNoName", "archivo sin nombre", "file with no name"),
    ("livery.invalidUrl", "URL inválida: {e}", "Invalid URL: {e}"),
    ("livery.joinPartsFailed", "No se pudieron unir las partes de la descarga. Tocá Cancelar y reintentá desde SimFleet.", "Couldn't join the download parts. Tap Cancel and retry from SimFleet."),
    ("livery.part", "parte {i}", "part {i}"),
    ("livery.partOf", "parte {i} de {total}", "part {i} of {total}"),
    ("livery.windowTitle", "Buscar liveries — flightsim.to (inicia sesión con tu cuenta)", "Browse liveries — flightsim.to (sign in with your account)"),
    ("manager.cancelledBeforeStart", "Cancelado por el usuario antes de empezar", "Cancelled by the user before starting"),
    ("manager.cancelledByUser", "Cancelado por el usuario", "Cancelled by the user"),
    ("manager.communityNotDetected", "No se detectó la carpeta Community de MSFS 2020. Configúrala manualmente desde Ajustes (próximamente) e intenta de nuevo.", "The MSFS 2020 Community folder was not detected. Set it manually from Settings (coming soon) and try again."),
    ("manager.connectingPeers", "Conectando con peers…", "Connecting to peers…"),
    ("manager.crashedDuringOpen", "La app se cerró mientras se abría el enlace en el navegador. Vuelve a pulsar el botón de descarga si lo necesitas.", "The app closed while opening the link in the browser. Press the download button again if you still need it."),
    ("manager.downloadOpened", "Descarga abierta dentro de SimFleet. Si el sitio pide un clic o un captcha, complétalo en la ventana; el archivo se instalará solo al terminar.", "Download opened inside SimFleet. If the site asks for a click or a captcha, complete it in the window; the file will install itself when it finishes."),
    ("manager.extracting", "Extrayendo archivo…", "Extracting archive…"),
    ("manager.hosterWindowTitle", "Descarga — SimFleet", "Download — SimFleet"),
    ("manager.installFailed", "Falló la instalación: {e}", "Installation failed: {e}"),
    ("manager.installTaskFailed", "La tarea de instalación falló: {err}", "The installation task failed: {err}"),
    ("manager.installedPackages", "Se instalaron {n} paquete(s) ({mb} MB)", "Installed {n} package(s) ({mb} MB)"),
    ("manager.magnetResolveFailed", "No se pudo obtener el enlace magnet del hoster: {e}", "Could not get the magnet link from the hoster: {e}"),
    ("manager.mgrUnavailable", "El gestor de descargas no está disponible", "The download manager is not available"),
    ("manager.noArchiveInTorrent", "El torrent no contiene un archivo .zip/.rar/.7z que podamos auto-instalar. Abre la carpeta de descargas para revisarlo.", "The torrent does not contain a .zip/.rar/.7z file we can auto-install. Open the downloads folder to check it."),
    ("manager.openDownloadFailed", "No se pudo abrir la descarga: {e}", "Could not open the download: {e}"),
    ("manager.openingDownload", "Abriendo la descarga dentro de SimFleet…", "Opening the download inside SimFleet…"),
    ("manager.openingDownloadParts", "Abriendo la descarga dentro de SimFleet ({n} partes)…", "Opening the download inside SimFleet ({n} parts)…"),
    ("manager.paused", "Pausado", "Paused"),
    ("manager.queuedWaiting", "En cola — esperando a que termine la descarga anterior…", "Queued — waiting for the previous download to finish…"),
    ("manager.resolvingMagnet", "Obteniendo enlace magnet (puede tardar ~10 s en SceneryAddons)…", "Getting the magnet link (may take ~10 s on SceneryAddons)…"),
    ("manager.torrentIsInstaller", "El torrent contiene un instalador (.exe/.msi), no un paquete auto-instalable. Descárgalo por el método Mirror/Direct: SimFleet lo abre y te deja guardarlo para ejecutarlo.", "The torrent contains an installer (.exe/.msi), not an auto-installable package. Download it via the Mirror/Direct method: SimFleet opens it and lets you save it to run."),
    ("packageOps.nothingToRemove", "no se encontró ninguna ubicación instalada para '{name}'", "no installed location was found for '{name}'"),
    ("packageOps.sanityNameMismatch", "el último componente '{name}' no coincide con el folder esperado '{expected}'", "the last component '{name}' does not match the expected folder '{expected}'"),
    ("packageOps.sanityNotCommunity", "el path no contiene 'community' ni 'sceneries' — sospechoso", "the path does not contain 'community' or 'sceneries' — suspicious"),
    ("packageOps.unknownPackage", "paquete '{name}' no encontrado en la base de datos", "package '{name}' not found in the database"),
    ("perf.enrichTaskFailed", "la tarea de enriquecimiento falló: {e}", "the enrichment task failed: {e}"),
    ("perf.noOptions", "Este escenario no tiene opciones de rendimiento.", "This scenery has no performance options."),
    ("perf.optionNotFound", "Opción '{option_id}' no encontrada.", "Option '{option_id}' not found."),
    ("perf.readTaskFailed", "la tarea de lectura falló: {e}", "the read task failed: {e}"),
    ("perf.renameFailed", "No se pudo renombrar {path}: {e}{hint}", "Could not rename {path}: {e}{hint}"),
    ("perf.renameLockedHint", " — MSFS tiene el escenario abierto. Cierra el simulador (o vuelve al menú principal) e inténtalo de nuevo.", " — MSFS has the scenery open. Close the simulator (or return to the main menu) and try again."),
    ("perf.scanTaskFailed", "la tarea de escaneo falló: {e}", "the scan task failed: {e}"),
    ("perf.sourceUnavailable", "fuente SceneryAddons no disponible", "SceneryAddons source not available"),
    ("perf.toggleTaskFailed", "la tarea de toggle falló: {e}", "the toggle task failed: {e}"),
    ("recorder.captureMonitorFailed", "no se pudo iniciar la captura del monitor: {e}", "could not start monitor capture: {e}"),
    ("recorder.captureStartWindowFailed", "no se pudo iniciar la captura de «{target}»: {e}", "could not start capture of «{target}»: {e}"),
    ("recorder.clipMoveFailed", "no se pudo mover el clip: {e}", "could not move the clip: {e}"),
    ("recorder.noFrames", "la captura no recibió ningún frame (¿ventana visible?)", "the capture received no frames (is the window visible?)"),
    ("recorder.noOutputFile", "la grabación no produjo ningún archivo", "the recording produced no file"),
    ("recorder.noWindowNoMonitor", "sin ventana «{target}» ni monitor primario: {e}", "no «{target}» window nor primary monitor: {e}"),
    ("recorder.windowsOnly", "la grabación solo está soportada en Windows", "recording is only supported on Windows"),
    ("recording.landingInProgress", "Hay una grabación de aterrizaje en curso. Inténtalo de nuevo cuando termine el vuelo.", "A landing recording is in progress. Try again when the flight ends."),
    ("recording.osdWindowNotFound", "ventana osd no encontrada", "osd window not found"),
    ("recording.taskFailed", "tarea de grabación falló: {e}", "recording task failed: {e}"),
    ("settings.autostartDisableFailed", "no se pudo desactivar autostart: {e}", "could not disable autostart: {e}"),
    ("settings.autostartEnableFailed", "no se pudo activar autostart: {e}", "could not enable autostart: {e}"),
    ("settings.invalidKey", "clave de setting no permitida: {key}", "setting key not allowed: {key}"),
    ("simbrief.configurePilotIdBriefing", "Configura tu SimBrief Pilot ID para ver el briefing.", "Set your SimBrief Pilot ID to see the briefing."),
    ("simbrief.configurePilotIdRefresh", "Configura tu SimBrief Pilot ID antes de refrescar.", "Set your SimBrief Pilot ID before refreshing."),
    ("simbrief.flightNotFound", "flight_log id={flightId} no existe", "flight_log id={flightId} does not exist"),
    ("simbrief.ofpNotFound", "OFP {ofpId} no existe en simbrief_flights", "OFP {ofpId} does not exist in simbrief_flights"),
    ("simbrief.pilotIdEmpty", "El pilot_id no puede estar vacío", "The pilot_id cannot be empty"),
    ("skybound.cloudflareBlocked", "Skybound bloqueó la petición (Cloudflare). Prueba de nuevo; si persiste, ábrelo en el navegador.", "Skybound blocked the request (Cloudflare). Try again; if it persists, open it in the browser."),
    ("skybound.direct", "Skybound (directo)", "Skybound (direct)"),
    ("skybound.download", "Descargar", "Download"),
    ("skybound.downloadParts", "{host} · descargar ({n} partes)", "{host} · download ({n} parts)"),
    ("skybound.namedParts", "{host} · {name} ({n} partes)", "{host} · {name} ({n} parts)"),
    ("skybound.openInSkybound", "Abrir en Skybound", "Open in Skybound"),
    ("torrent.cancelledResolvingMetadata", "cancelado mientras se resolvía la metadata del magnet", "cancelled while resolving the magnet metadata"),
    ("torrent.metadataTimeout", "timeout resolviendo la metadata del magnet (¿sin seeds?)", "timed out resolving the magnet metadata (no seeds?)"),
    ("updater.autoInstallWindowsOnly", "auto-install sólo soportado en Windows", "auto-install is only supported on Windows"),
    ("updater.chunkWriteFailed", "no se pudo escribir el chunk: {e}", "could not write the chunk: {e}"),
    ("updater.currentExeFailed", "no se pudo resolver el ejecutable actual: {e}", "could not resolve the current executable: {e}"),
    ("updater.downloadFailed", "descarga falló: {e}", "download failed: {e}"),
    ("updater.downloadFailedHttp", "descarga falló con HTTP {status}", "download failed with HTTP {status}"),
    ("updater.downloadInterrupted", "descarga interrumpida: {e}", "download interrupted: {e}"),
    ("updater.launchInstallerFailed", "no se pudo lanzar el instalador: {e}", "could not launch the installer: {e}"),
    ("updater.tempCreateFailed", "no se pudo crear el archivo temporal: {e}", "could not create the temporary file: {e}"),
    ("updater.urlNotGithub", "La URL de actualización no es de GitHub; cancelado por seguridad.", "The update URL is not from GitHub; cancelled for security."),
];
