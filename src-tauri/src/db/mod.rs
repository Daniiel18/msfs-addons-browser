use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{ConnectOptions, SqlitePool};

pub async fn init(app_data_dir: &Path) -> anyhow::Result<SqlitePool> {
    let db_path = app_data_dir.join("msfs-addons.db");
    tracing::info!("opening database at {}", db_path.display());

    let opts = SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true)
        .log_statements(tracing::log::LevelFilter::Debug);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .map_err(|e| {
            tracing::error!("db: pool connect_with falló: {e:#}");
            e
        })?;
    tracing::info!("db: pool conectado, corriendo migraciones…");

    // (v3.7.1 hotfix) Normaliza checksums de migraciones ya aplicadas.
    //
    // **Bug**: usuario reportó "splash blanco y se cierra" tras upgrade
    // v3.5.0→v3.7.0. El log terminaba en "opening database" sin pista.
    // Diagnóstico via binario `diag_migrate` reveló:
    //   `migration 1 was previously applied but has been modified`
    //
    // **Causa raíz**: las migraciones SQL fueron movidas entre repos
    // (worktree → clean clone) y Git autocrlf convirtió LF→CRLF. El
    // checksum que sqlx calcula sobre el contenido embedded en el
    // binario v3.7.0 ya NO coincide con el guardado en `_sqlx_migrations`
    // (calculado por v3.5.0 sobre el archivo LF original).
    //
    // Las migraciones son funcionalmente idénticas — sólo cambió el
    // line ending. Reaplicarlas fallaría (las columnas/tablas ya
    // existen). La solución segura: UPDATE de los checksums almacenados
    // para que coincidan con el binario actual.
    //
    // Esto es un parche de compatibilidad. Si en el futuro alguna
    // migración cambia REALMENTE su contenido, este normalize la
    // bypasearía silenciosamente — pero el contrato es que las
    // migraciones son inmutables después de release, así que no
    // debería pasar.
    if let Err(e) = normalize_migration_checksums(&pool).await {
        tracing::warn!(
            "db: normalize_migration_checksums falló (no-op si tabla no existe aún): {e:#}"
        );
    }

    // (v6.2.24) BACKUP automático de la DB antes de aplicar migraciones
    // NUEVAS. El historial de vuelos es valioso (cientos de miles de puntos
    // de track); si una migración saliera mal, `msfs-addons.db.bak` permite
    // recuperar. Solo se copia cuando de verdad hay migraciones pendientes
    // (no en cada arranque) y tras un checkpoint del WAL para que la copia
    // sea consistente. Un solo .bak rotativo (el del último upgrade).
    let migrator = sqlx::migrate!("./migrations");
    let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);
    let total = migrator.iter().count() as i64;
    if applied > 0 && applied < total {
        let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&pool)
            .await;
        let backup = db_path.with_extension("db.bak");
        match std::fs::copy(&db_path, &backup) {
            Ok(bytes) => tracing::info!(
                "db: backup pre-migración escrito ({:.1} MB) en {} — {} migraciones nuevas por aplicar",
                bytes as f64 / 1_048_576.0,
                backup.display(),
                total - applied
            ),
            Err(e) => tracing::warn!("db: backup pre-migración falló (se continúa): {e:#}"),
        }
    }

    // (v3.7.1) Migración con logging explícito por si falla — sin
    // esto un fallo se propagaba silencioso (el `?` mata el setup de
    // Tauri sin que veamos qué pasó).
    match migrator.run(&pool).await {
        Ok(()) => tracing::info!("db: migraciones aplicadas OK"),
        Err(e) => {
            tracing::error!("db: sqlx migrate FALLÓ: {e:#}");
            return Err(e.into());
        }
    }

    // (v4.0.2) Mantenimiento automático al arrancar — ventana segura
    // (el watcher aún no escribe). NO destructivo: diagnóstico + libera
    // el WAL (que llegaba a cientos de MB y hacía cada conexión lenta) +
    // configura auto-checkpoint para que no vuelva a inflarse.
    run_db_maintenance(&pool).await;

    // (v3.24.0) Tras el dedup masivo de track points (migración 033) la
    // DB queda con muchas páginas libres. Las recuperamos con VACUUM —
    // que NO puede correr dentro de una transacción, por eso va acá
    // (fuera de la migración sqlx) y no dentro del .sql.
    if let Err(e) = vacuum_if_bloated(&pool).await {
        tracing::warn!("db: vacuum_if_bloated falló (no fatal): {e:#}");
    }

    Ok(pool)
}

/// (v4.0.2) Mantenimiento automático de la base de datos al arrancar.
///
/// Corre en `init()`, ANTES de que el watcher de SimConnect empiece a
/// escribir — ventana segura sin escritores concurrentes. Es **no
/// destructivo**: no borra ni un solo vuelo ni track point del usuario.
///
///   1. **Diagnóstico**: tamaño real de la DB, espacio libre, filas por
///      tabla y top vuelos por nº de track points. Esto revela QUÉ está
///      inflando la DB (típicamente imports VAS-ACARS muy densos) para
///      poder diseñar después una poda PRECISA y segura.
///   2. **Checkpoint del WAL (TRUNCATE)**: vuelca el `.db-wal` al archivo
///      principal y lo trunca a 0. El usuario tenía un WAL de ~211 MB que
///      hacía que cada conexión del pool tardara >2 s en adquirirse
///      (warnings `slow_acquire`). Esto también reduce la contención de
///      disco con MSFS (que streamea escenario del mismo disco).
///   3. **wal_autocheckpoint**: mantiene el WAL pequeño (~4 MB) de aquí
///      en adelante para que no se vuelva a inflar.
///   4. **optimize**: refresca estadísticas de índices.
async fn run_db_maintenance(pool: &SqlitePool) {
    if let Err(e) = log_db_diagnostics(pool).await {
        tracing::warn!("db: diagnóstico falló (no fatal): {e:#}");
    }
    match sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await
    {
        Ok(_) => tracing::info!("db: WAL checkpoint(TRUNCATE) OK — WAL liberado"),
        Err(e) => tracing::warn!("db: WAL checkpoint falló (no fatal): {e:#}"),
    }
    // ~1000 páginas × 4 KB ≈ 4 MB de WAL máximo antes de auto-checkpoint.
    let _ = sqlx::query("PRAGMA wal_autocheckpoint = 1000")
        .execute(pool)
        .await;
    let _ = sqlx::query("PRAGMA optimize").execute(pool).await;
}

/// (v4.0.2) Loguea métricas de tamaño de la DB y conteos por tabla.
/// Pensado para que, viendo el `simfleet.log`, sepamos exactamente qué
/// tabla/vuelo está inflando la base — sin adivinar ni borrar a ciegas.
async fn log_db_diagnostics(pool: &SqlitePool) -> anyhow::Result<()> {
    let page_count: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let freelist: i64 = sqlx::query_scalar("PRAGMA freelist_count")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    tracing::info!(
        "db diag: tamaño={:.1} MB (page_count={}, page_size={}) · libre={:.1} MB ({} páginas)",
        (page_count * page_size) as f64 / 1_048_576.0,
        page_count,
        page_size,
        (freelist * page_size) as f64 / 1_048_576.0,
        freelist,
    );
    for table in [
        "flight_log",
        "flight_log_track",
        "flight_log_phase",
        "score_item",
        "addons",
        "community_packages",
    ] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(pool)
            .await
            .unwrap_or(-1);
        tracing::info!("db diag: {} → {} filas", table, n);
    }
    let top: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT flight_id, COUNT(*) c FROM flight_log_track GROUP BY flight_id ORDER BY c DESC LIMIT 5",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (fid, c) in top {
        tracing::info!("db diag: vuelo {} → {} track points", fid, c);
    }
    // Muestra de timestamps — necesaria para conocer el formato exacto
    // ANTES de cualquier poda por tiempo (evita borrar lo que no toca).
    let sample: Vec<(String,)> =
        sqlx::query_as("SELECT ts FROM flight_log_track ORDER BY id DESC LIMIT 3")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    for (ts,) in sample {
        tracing::info!("db diag: muestra ts='{}'", ts);
    }
    Ok(())
}

/// (v3.24.0) Corre `VACUUM` una sola vez si el archivo tiene mucho
/// espacio libre (típico tras el dedup masivo de la migración 033 que
/// borró los track points duplicados). Sólo se dispara con freelist
/// grande (~40 MB); tras compactar, el freelist baja y no se repite en
/// arranques siguientes, así que no penaliza el inicio normal.
async fn vacuum_if_bloated(pool: &SqlitePool) -> anyhow::Result<()> {
    let freelist: i64 = sqlx::query_scalar("PRAGMA freelist_count")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    const THRESHOLD_PAGES: i64 = 10_000; // ~40 MB con páginas de 4 KB
    if freelist > THRESHOLD_PAGES {
        tracing::info!(
            "db: {} páginas libres (> {}) → VACUUM para recuperar disco",
            freelist,
            THRESHOLD_PAGES
        );
        sqlx::query("VACUUM").execute(pool).await?;
        tracing::info!("db: VACUUM completado");
    }
    Ok(())
}

/// (v3.7.1 hotfix) Normaliza los checksums de `_sqlx_migrations` para
/// que coincidan con los embedded en el binario actual. Bypass del
/// check de "previously applied but has been modified" cuando la
/// causa es line endings (CRLF↔LF) o whitespace cosmético.
///
/// Sólo toca rows que YA existen en `_sqlx_migrations` y que tienen
/// el mismo `version` que una migración embedded — el contenido
/// asumimos funcionalmente idéntico.
///
/// Si `_sqlx_migrations` no existe todavía (instalación fresca),
/// devuelve Ok sin tocar nada — `sqlx::migrate!` la creará.
async fn normalize_migration_checksums(pool: &SqlitePool) -> anyhow::Result<()> {
    // ¿Existe la tabla?
    let table_exists: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
    )
    .fetch_optional(pool)
    .await?;
    if table_exists.is_none() {
        tracing::debug!("db: _sqlx_migrations no existe — primera instalación");
        return Ok(());
    }

    // Itera las migraciones embedded en el binario.
    let migrator = sqlx::migrate!("./migrations");
    let mut updated = 0;
    for migration in migrator.iter() {
        let version = migration.version;
        let new_checksum: &[u8] = &migration.checksum;
        // ¿Está aplicada ya con un checksum distinto?
        let stored: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT checksum FROM _sqlx_migrations WHERE version = ?1",
        )
        .bind(version)
        .fetch_optional(pool)
        .await?;
        let Some(stored) = stored else { continue };
        if stored.as_slice() == new_checksum {
            continue;
        }
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ?1 WHERE version = ?2")
            .bind(new_checksum)
            .bind(version)
            .execute(pool)
            .await?;
        updated += 1;
        tracing::info!(
            "db: checksum de migración {} ({}) normalizado — line endings o whitespace cambió",
            version,
            migration.description
        );
    }
    if updated > 0 {
        tracing::info!(
            "db: normalize_migration_checksums OK ({} migraciones actualizadas)",
            updated
        );
    } else {
        tracing::debug!("db: todos los checksums coinciden, sin cambios");
    }
    Ok(())
}

pub mod repo {
    use sqlx::SqlitePool;

    use crate::download::DownloadJob;
    use crate::sources::Addon;

    pub async fn upsert_addon(pool: &SqlitePool, addon: &Addon) -> anyhow::Result<()> {
        // `released_at` se actualiza con `COALESCE(?, released_at)` —
        // si el scraper no extrajo fecha esta vez, conservamos la
        // anterior. Algunos posts viejos pierden el `<time>` cuando
        // el theme cambia.
        sqlx::query(
            r#"
            INSERT INTO addons (id, source, title, developer, name, version, icao, simulator, page_url, released_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                developer = excluded.developer,
                name = excluded.name,
                version = excluded.version,
                icao = excluded.icao,
                simulator = excluded.simulator,
                page_url = excluded.page_url,
                released_at = COALESCE(excluded.released_at, addons.released_at),
                last_seen_at = datetime('now')
            "#,
        )
        .bind(&addon.id)
        .bind(&addon.source)
        .bind(&addon.title)
        .bind(&addon.developer)
        .bind(&addon.name)
        .bind(&addon.version)
        .bind(&addon.icao)
        .bind(&addon.simulator)
        .bind(&addon.page_url)
        .bind(&addon.released_at)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// (v5.0.0) Devuelve `(page_url, icao)` del addon de catálogo por su
    /// id — para que la descarga pueda bajar la nota "Optional
    /// Configuration" de su página al instalar.
    pub async fn get_addon_page(
        pool: &SqlitePool,
        addon_id: &str,
    ) -> anyhow::Result<Option<(String, Option<String>)>> {
        let row: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT page_url, icao FROM addons WHERE id = ?1")
                .bind(addon_id)
                .fetch_optional(pool)
                .await?;
        Ok(row.filter(|(url, _)| !url.is_empty()))
    }

    /// Contexto necesario para registrar una instalación en DB. Los
    /// campos opcionales se usan cuando tenemos un addon-origen
    /// (descarga por torrent desde una fuente conocida); para una
    /// instalación manual basta con `title` + `install_path` + `size`.
    #[derive(Debug, Clone)]
    pub struct InstallRecord {
        pub id: String,
        pub addon_id: Option<String>,
        pub source: Option<String>,
        pub title: String,
        pub name: Option<String>,
        pub developer: Option<String>,
        pub version: Option<String>,
        pub install_path: String,
        pub size_bytes: Option<i64>,
    }

    /// Inserta una fila nueva en `installed_addons`. No hace upsert: cada
    /// instalación es un evento independiente, aunque se sobrescriba una
    /// previa del mismo addon — eso nos deja ver el historial completo.
    pub async fn record_install(pool: &SqlitePool, rec: &InstallRecord) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO installed_addons (
                id, addon_id, source, title, name, developer, version,
                install_path, size_bytes
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(&rec.id)
        .bind(&rec.addon_id)
        .bind(&rec.source)
        .bind(&rec.title)
        .bind(&rec.name)
        .bind(&rec.developer)
        .bind(&rec.version)
        .bind(&rec.install_path)
        .bind(&rec.size_bytes)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Elimina una fila del historial. No toca el disco — el borrado
    /// físico de la carpeta en Community es un paso aparte (y por
    /// ahora opcional, confirmado por el usuario).
    pub async fn forget_install(pool: &SqlitePool, id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM installed_addons WHERE id = ?1")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Fila enriquecida que viaja al frontend. Los campos adicionales
    /// (vs la definición antigua) son los denormalizados en la migración
    /// 002 para que las instalaciones manuales — sin fila en `addons` —
    /// también se puedan mostrar sin JOIN.
    #[derive(Debug, serde::Serialize, sqlx::FromRow)]
    #[serde(rename_all = "camelCase")]
    pub struct InstalledAddonRow {
        pub id: String,
        pub addon_id: Option<String>,
        pub source: Option<String>,
        pub title: String,
        pub name: Option<String>,
        pub developer: Option<String>,
        pub version: Option<String>,
        pub install_path: String,
        pub size_bytes: Option<i64>,
        pub installed_at: String,
    }

    pub async fn list_installed(pool: &SqlitePool) -> anyhow::Result<Vec<InstalledAddonRow>> {
        let rows = sqlx::query_as::<_, InstalledAddonRow>(
            r#"
            SELECT id, addon_id, source, title, name, developer, version,
                   install_path, size_bytes, installed_at
            FROM installed_addons
            ORDER BY installed_at DESC
            "#,
        )
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    /// Fila bruta de la caché de GSX. Devolvemos el JSON sin parsear
    /// porque la deserialización vive en `gsx::cache_get` — así el
    /// repo no depende del struct concreto del scraper.
    #[derive(Debug, sqlx::FromRow)]
    pub struct GsxCacheRow {
        pub profiles_json: String,
        pub fetched_at: String,
    }

    pub async fn get_gsx_lookup(
        pool: &SqlitePool,
        icao: &str,
    ) -> anyhow::Result<Option<GsxCacheRow>> {
        let row = sqlx::query_as::<_, GsxCacheRow>(
            r#"SELECT profiles_json, fetched_at FROM gsx_lookups WHERE icao = ?1"#,
        )
        .bind(icao)
        .fetch_optional(pool)
        .await?;
        Ok(row)
    }

    /// Inserta o reemplaza la entrada de caché para `icao`. El payload
    /// debe ser un JSON serializado de `Vec<GsxProfile>` — vacío vale
    /// como negative cache para no reintentar contra flightsim.to.
    pub async fn set_gsx_lookup(
        pool: &SqlitePool,
        icao: &str,
        profiles_json: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO gsx_lookups (icao, profiles_json, fetched_at)
            VALUES (?1, ?2, datetime('now'))
            ON CONFLICT(icao) DO UPDATE SET
                profiles_json = excluded.profiles_json,
                fetched_at = excluded.fetched_at
            "#,
        )
        .bind(icao)
        .bind(profiles_json)
        .execute(pool)
        .await?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Community packages
    // -----------------------------------------------------------------

    /// Fila enriquecida con coordenadas resueltas (cuando el ICAO
    /// del paquete existe en `airports`). Es lo que pinta el mapa.
    /// Usamos Option para lat/lon porque hay paquetes legítimos sin
    /// ICAO conocido (livery packs, sound packs) — siguen apareciendo
    /// en la lista lateral pero no en el mapa.
    #[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
    #[serde(rename_all = "camelCase")]
    pub struct CommunityPackageRow {
        pub folder_name: String,
        pub install_path: String,
        pub title: String,
        pub creator: Option<String>,
        pub content_type: Option<String>,
        pub package_version: Option<String>,
        pub minimum_game_version: Option<String>,
        pub icao: Option<String>,
        pub size_bytes: Option<i64>,
        pub folder_modified_at: Option<String>,
        pub dependencies_count: i64,
        pub scanned_at: String,
        /// (v4.25.0) Estado del paquete en el sim. `false` = el toggle
        /// renombró layout.json → layout.json.disabled y MSFS lo
        /// ignora. El scanner lo recalcula del FS en cada scan.
        pub enabled: bool,
        /// (v4.28.0) Contenedores SimObjects/Airplanes propios del
        /// paquete (JSON-encoded). El frontend los necesita para
        /// distinguir un avión real (con sus propios containers) de
        /// una livery (sin containers o con base_container fuera).
        #[sqlx(default)]
        pub simobject_dirs_json: String,
        /// (v4.28.0) Valores de `base_container` del aircraft.cfg
        /// (último componente del path, JSON-encoded).
        #[sqlx(default)]
        pub base_containers_json: String,
        /// (v4.31.0) true si tiene geometría 3D propia (avión real vs
        /// modificación de texturas/cabina).
        pub has_own_model: bool,
        /// (v4.31.0) false si el paquete no trae manifest.json (3rd-party).
        pub has_manifest: bool,
        /// (v4.29.0) Ruta absoluta del thumbnail resuelto durante el
        /// scan (`<pkg>/thumbnail.jpg`, `SimObjects/Airplanes/*/…`, o
        /// `layout.json` fallback). NULL → frontend pinta silueta
        /// premium. `package_thumbnail` lee este path: O(1) en runtime.
        #[sqlx(default)]
        pub thumbnail_path: Option<String>,
        pub airport_name: Option<String>,
        pub latitude: Option<f64>,
        pub longitude: Option<f64>,
        /// (v4.24.1) Computado en Rust (no es columna): true si el
        /// paquete parece librería/complemento (AIRAC, night lights,
        /// enhancements, excludes…). El mapa lo usa para NO pintarlo y
        /// el dashboard para no contarlo — misma lógica para ambos.
        #[sqlx(default)]
        pub is_library_pack: bool,
    }

    pub async fn list_community_packages(
        pool: &SqlitePool,
    ) -> anyhow::Result<Vec<CommunityPackageRow>> {
        let rows = sqlx::query_as::<_, CommunityPackageRow>(
            r#"
            SELECT cp.folder_name,
                   cp.install_path,
                   cp.title,
                   cp.creator,
                   cp.content_type,
                   cp.package_version,
                   cp.minimum_game_version,
                   cp.icao,
                   cp.size_bytes,
                   cp.folder_modified_at,
                   cp.dependencies_count,
                   cp.scanned_at,
                   cp.enabled,
                   cp.simobject_dirs_json,
                   cp.base_containers_json,
                   cp.thumbnail_path,
                   cp.has_own_model,
                   cp.has_manifest,
                   ap.name      AS airport_name,
                   ap.latitude  AS latitude,
                   ap.longitude AS longitude
            FROM community_packages cp
            LEFT JOIN airports ap ON ap.icao = UPPER(cp.icao)
            ORDER BY cp.title COLLATE NOCASE ASC
            "#,
        )
        .fetch_all(pool)
        .await?;
        // (v4.24.1) Clasificación library/complemento computada acá —
        // única fuente de verdad compartida con el dashboard.
        let mut rows = rows;
        for r in &mut rows {
            r.is_library_pack =
                crate::community_scanner::is_library_pack(&r.title, &r.folder_name);
        }
        Ok(rows)
    }

    // -----------------------------------------------------------------
    // (v4.25.0) Link Map — aristas y posiciones del grafo de addons
    // -----------------------------------------------------------------

    /// Arista dirigida del grafo de dependencias: `source` (paquete
    /// base, p.ej. el aircraft) → `target` (dependiente: livery,
    /// soundpack, tweak). `origin` = 'auto' (sembrada del manifest)
    /// o 'manual' (creada por el usuario en el Link Map).
    #[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
    #[serde(rename_all = "camelCase")]
    pub struct AddonLinkRow {
        pub id: i64,
        pub source_folder: String,
        pub target_folder: String,
        pub origin: String,
    }

    pub async fn list_addon_links(pool: &SqlitePool) -> anyhow::Result<Vec<AddonLinkRow>> {
        let rows = sqlx::query_as::<_, AddonLinkRow>(
            "SELECT id, source_folder, target_folder, origin FROM addon_links ORDER BY id",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    pub async fn add_addon_link(
        pool: &SqlitePool,
        source_folder: &str,
        target_folder: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO addon_links (source_folder, target_folder, origin)
            VALUES (?1, ?2, 'manual')
            "#,
        )
        .bind(source_folder)
        .bind(target_folder)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn remove_addon_link(
        pool: &SqlitePool,
        source_folder: &str,
        target_folder: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "DELETE FROM addon_links WHERE source_folder = ?1 AND target_folder = ?2",
        )
        .bind(source_folder)
        .bind(target_folder)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Posición persistida de un nodo en el lienzo del Link Map.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
    #[serde(rename_all = "camelCase")]
    pub struct AddonNodePosition {
        pub folder_name: String,
        pub x: f64,
        pub y: f64,
    }

    pub async fn list_addon_node_positions(
        pool: &SqlitePool,
    ) -> anyhow::Result<Vec<AddonNodePosition>> {
        let rows = sqlx::query_as::<_, AddonNodePosition>(
            "SELECT folder_name, x, y FROM addon_link_positions",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    pub async fn save_addon_node_positions(
        pool: &SqlitePool,
        positions: &[AddonNodePosition],
    ) -> anyhow::Result<()> {
        let mut tx = pool.begin().await?;
        for p in positions {
            sqlx::query(
                r#"
                INSERT INTO addon_link_positions (folder_name, x, y)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(folder_name) DO UPDATE SET x = excluded.x, y = excluded.y
                "#,
            )
            .bind(&p.folder_name)
            .bind(p.x)
            .bind(p.y)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Devuelve los ICAO únicos detectados en Community que tienen
    /// `package_version`. Es la lista que el detector de actualizaciones
    /// usa para ir a interrogar las fuentes.
    pub async fn community_icaos_with_version(
        pool: &SqlitePool,
    ) -> anyhow::Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            r#"
            SELECT DISTINCT UPPER(icao) FROM community_packages
            WHERE icao IS NOT NULL AND icao <> ''
              AND package_version IS NOT NULL AND package_version <> ''
              AND UPPER(content_type) = 'SCENERY'
            "#,
        )
        .fetch_all(pool)
        .await?;
        Ok(rows.into_iter().map(|(s,)| s).collect())
    }

    /// Términos de búsqueda extra para refrescar updates de
    /// AIRCRAFT/INSTRUMENT/MISC. Como esos no tienen ICAO, usamos
    /// la primera palabra distintiva del título — suficiente para
    /// que el scraper de Simplaza/SceneryAddons devuelva el addon
    /// correcto cuando existe.
    pub async fn community_addon_keywords_with_version(
        pool: &SqlitePool,
    ) -> anyhow::Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            r#"
            SELECT DISTINCT title FROM community_packages
            WHERE package_version IS NOT NULL AND package_version <> ''
              AND UPPER(content_type) IN ('AIRCRAFT', 'INSTRUMENT', 'MISC')
            "#,
        )
        .fetch_all(pool)
        .await?;
        let mut keywords: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for (title,) in rows {
            for word in title.split_whitespace() {
                let trimmed = word
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_string();
                if trimmed.len() >= 4 && trimmed.chars().any(|c| c.is_alphabetic()) {
                    keywords.insert(trimmed);
                    break;
                }
            }
        }
        Ok(keywords.into_iter().collect())
    }

    /// Una update detectada: el paquete local + la mejor versión que
    /// hay en el catálogo (`addons.version`) para el mismo ICAO.
    /// La comparación de versiones la hacemos en Rust (no SQL) porque
    /// SQLite no entiende semver.
    #[derive(Debug, Clone, sqlx::FromRow)]
    pub struct UpdateCandidate {
        pub folder_name: String,
        pub title: String,
        pub icao: String,
        pub installed_version: String,
        pub catalog_version: String,
        pub catalog_source: String,
        pub catalog_addon_id: String,
        pub catalog_page_url: String,
        /// (v4.16.3) Creator del manifest + developer del catálogo — el match
        /// creator↔developer se hace AHORA en Rust (`compute_available`)
        /// normalizando a solo alfanuméricos, robusto ante guiones Unicode
        /// (en-dash "–" del catálogo vs "-" ASCII del manifest, etc.).
        #[sqlx(default)]
        pub creator: String,
        #[sqlx(default)]
        pub developer: String,
    }

    // -----------------------------------------------------------------
    // Cache de update checks
    // -----------------------------------------------------------------

    #[derive(Debug, Clone, sqlx::FromRow)]
    pub struct UpdateCheckCacheRow {
        pub icao: String,
        pub source: String,
        pub last_known_version: Option<String>,
        pub catalog_addon_id: Option<String>,
        pub checked_at: String,
    }

    pub async fn get_update_check_cache(
        pool: &SqlitePool,
        icao: &str,
        source: &str,
    ) -> anyhow::Result<Option<UpdateCheckCacheRow>> {
        let row = sqlx::query_as::<_, UpdateCheckCacheRow>(
            r#"
            SELECT icao, source, last_known_version, catalog_addon_id, checked_at
            FROM update_check_cache
            WHERE icao = ?1 AND source = ?2
            "#,
        )
        .bind(icao)
        .bind(source)
        .fetch_optional(pool)
        .await?;
        Ok(row)
    }

    pub async fn set_update_check_cache(
        pool: &SqlitePool,
        icao: &str,
        source: &str,
        last_known_version: Option<&str>,
        catalog_addon_id: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO update_check_cache (icao, source, last_known_version, catalog_addon_id, checked_at)
            VALUES (?1, ?2, ?3, ?4, datetime('now'))
            ON CONFLICT(icao, source) DO UPDATE SET
                last_known_version = excluded.last_known_version,
                catalog_addon_id   = excluded.catalog_addon_id,
                checked_at         = excluded.checked_at
            "#,
        )
        .bind(icao)
        .bind(source)
        .bind(last_known_version)
        .bind(catalog_addon_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Devuelve **todos** los matches catalog ↔ community por ICAO.
    /// Un mismo `folder_name` puede aparecer N veces (uno por cada
    /// addon en el catálogo con ese ICAO). El caller (`updates.rs`)
    /// agrupa por folder y se queda con el de **mayor versión** —
    /// no con el más reciente, que es lo que daba notificaciones
    /// equivocadas (la última fuente buscada ganaba aunque tuviera
    /// versión inferior).
    // -----------------------------------------------------------------
    // Updates descartadas por el usuario ("marcar como vista")
    // -----------------------------------------------------------------

    /// Inserta o reemplaza una entrada — una update queda oculta
    /// hasta que el usuario pulse "Recargar", la instale, o salga una
    /// VERSIÓN MÁS NUEVA que la descartada (v6.2.22: el descarte guarda
    /// la versión del catálogo; descartar v1.0.2 no silencia v1.0.3).
    pub async fn dismiss_update(
        pool: &SqlitePool,
        folder_name: &str,
        latest_version: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO dismissed_updates (folder_name, latest_version, dismissed_at)
            VALUES (?1, ?2, datetime('now'))
            ON CONFLICT(folder_name) DO UPDATE
                SET dismissed_at = excluded.dismissed_at,
                    latest_version = excluded.latest_version
            "#,
        )
        .bind(folder_name)
        .bind(latest_version)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Limpia todas las descartadas — el panel de notificaciones
    /// llama a esto antes de cada refresh manual para que las
    /// pendientes siempre vuelvan a aparecer.
    pub async fn clear_dismissed_updates(pool: &SqlitePool) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM dismissed_updates")
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Devuelve folder_name → versión descartada. Lo cargamos una
    /// vez en `compute_available` y filtramos en memoria — más
    /// barato que JOIN cuando hay sólo decenas de filas.
    pub async fn list_dismissed_versions(
        pool: &SqlitePool,
    ) -> anyhow::Result<std::collections::HashMap<String, String>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT folder_name, latest_version FROM dismissed_updates")
                .fetch_all(pool)
                .await?;
        Ok(rows.into_iter().collect())
    }

    // -----------------------------------------------------------------
    // Diagnóstico de updates
    // -----------------------------------------------------------------

    /// Snapshot estructurado de toda la cadena de detección de
    /// updates para un paquete concreto. Lo expone un comando Tauri
    /// para que la UI pueda mostrar exactamente qué eslabón está
    /// fallando — sin pedirle al usuario que mire logs ni edite
    /// SQL a mano.
    #[derive(Debug, Clone, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct UpdateDiagnostic {
        pub folder_name: String,
        /// Filas de `community_packages` para ese folder. Si vacío,
        /// el paquete ni siquiera está escaneado en DB.
        pub package: Option<DiagPackage>,
        /// Match en la tabla `airports` por el ICAO del paquete.
        /// `None` si el ICAO no está en OurAirports (o el paquete
        /// no tiene ICAO).
        pub airport_match: Option<DiagAirport>,
        /// Filas de `addons` con el mismo ICAO. Sirve para ver qué
        /// versiones tiene el catálogo cacheadas.
        pub catalog_entries: Vec<DiagCatalog>,
        /// Filas en `update_check_cache` para este ICAO — útil para
        /// ver si el refresh está saltándose la query por TTL.
        pub cache_entries: Vec<DiagCache>,
        /// Razón por la cual la update **no** está apareciendo, en
        /// lenguaje humano. `None` cuando sí hay update visible.
        pub blocker: Option<String>,
        /// Si todo está bien y hay update, los detalles que se
        /// emitirían en el panel de notificaciones.
        pub would_emit: Option<DiagWouldEmit>,
    }

    #[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
    #[serde(rename_all = "camelCase")]
    pub struct DiagPackage {
        pub icao: Option<String>,
        pub package_version: Option<String>,
        pub content_type: Option<String>,
        pub title: String,
    }

    #[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
    #[serde(rename_all = "camelCase")]
    pub struct DiagAirport {
        pub icao: String,
        pub name: String,
    }

    #[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
    #[serde(rename_all = "camelCase")]
    pub struct DiagCatalog {
        pub source: String,
        pub addon_id: String,
        pub title: String,
        pub version: Option<String>,
        pub last_seen_at: String,
    }

    #[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
    #[serde(rename_all = "camelCase")]
    pub struct DiagCache {
        pub source: String,
        pub last_known_version: Option<String>,
        pub checked_at: String,
    }

    #[derive(Debug, Clone, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct DiagWouldEmit {
        pub installed_version: String,
        pub latest_version: String,
        pub source: String,
    }

    /// Construye el diagnóstico completo para `folder_name`. Hace
    /// varias queries — está pensado para invocación on-demand
    /// (botón en el modal), no para correr en cada render.
    pub async fn diagnose_for_folder(
        pool: &SqlitePool,
        folder_name: &str,
    ) -> anyhow::Result<UpdateDiagnostic> {
        // 1. Paquete en community_packages
        let package: Option<DiagPackage> = sqlx::query_as::<_, DiagPackage>(
            r#"
            SELECT icao, package_version, content_type, title
            FROM community_packages
            WHERE folder_name = ?1
            "#,
        )
        .bind(folder_name)
        .fetch_optional(pool)
        .await?;

        let icao_upper = package
            .as_ref()
            .and_then(|p| p.icao.as_ref())
            .map(|s| s.trim().to_ascii_uppercase())
            .filter(|s| !s.is_empty());

        // 2. Airport match
        let airport_match: Option<DiagAirport> = if let Some(icao) = icao_upper.as_deref() {
            sqlx::query_as::<_, DiagAirport>(
                "SELECT icao, name FROM airports WHERE icao = ?1",
            )
            .bind(icao)
            .fetch_optional(pool)
            .await?
        } else {
            None
        };

        // 3. Catalog entries por ICAO
        let catalog_entries: Vec<DiagCatalog> = if let Some(icao) = icao_upper.as_deref() {
            sqlx::query_as::<_, DiagCatalog>(
                r#"
                SELECT source, id AS addon_id, title, version, last_seen_at
                FROM addons
                WHERE UPPER(icao) = ?1
                ORDER BY last_seen_at DESC
                "#,
            )
            .bind(icao)
            .fetch_all(pool)
            .await?
        } else {
            Vec::new()
        };

        // 4. Cache entries por ICAO
        let cache_entries: Vec<DiagCache> = if let Some(icao) = icao_upper.as_deref() {
            sqlx::query_as::<_, DiagCache>(
                r#"
                SELECT source, last_known_version, checked_at
                FROM update_check_cache
                WHERE UPPER(icao) = ?1
                ORDER BY checked_at DESC
                "#,
            )
            .bind(icao)
            .fetch_all(pool)
            .await?
        } else {
            Vec::new()
        };

        // 5. Determinar blocker
        let mut blocker: Option<String> = None;
        let mut would_emit: Option<DiagWouldEmit> = None;

        if package.is_none() {
            blocker = Some(crate::tr!("db.diagPackageMissing", folder = folder_name));
        } else if let Some(pkg) = package.as_ref() {
            if pkg.icao.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true) {
                blocker = Some(crate::tr!("db.diagNoIcao"));
            } else if pkg
                .content_type
                .as_deref()
                .map(|s| !s.eq_ignore_ascii_case("SCENERY"))
                .unwrap_or(true)
            {
                let empty = crate::tr!("db.emptyValue");
                blocker = Some(crate::tr!(
                    "db.diagWrongContentType",
                    contentType = pkg.content_type.as_deref().unwrap_or(&empty)
                ));
            } else if pkg
                .package_version
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                blocker = Some(crate::tr!("db.diagNoVersion"));
            } else if airport_match.is_none() {
                blocker = Some(crate::tr!(
                    "db.diagIcaoNotInAirports",
                    icao = icao_upper.as_deref().unwrap_or("?")
                ));
            } else {
                let with_version: Vec<&DiagCatalog> = catalog_entries
                    .iter()
                    .filter(|e| e.version.as_ref().map(|v| !v.trim().is_empty()).unwrap_or(false))
                    .collect();

                if with_version.is_empty() {
                    blocker = Some(crate::tr!(
                        "db.diagNoCatalogVersion",
                        icao = icao_upper.as_deref().unwrap_or("?")
                    ));
                } else {
                    // Pick max version (string compare lenient — el caller
                    // tiene la lógica real, aquí la simplificamos para
                    // diagnostico).
                    let installed = pkg.package_version.clone().unwrap_or_default();
                    let best = with_version
                        .iter()
                        .max_by(|a, b| {
                            a.version
                                .as_deref()
                                .unwrap_or("")
                                .cmp(b.version.as_deref().unwrap_or(""))
                        })
                        .unwrap();
                    let latest = best.version.clone().unwrap_or_default();
                    if latest <= installed {
                        blocker = Some(crate::tr!(
                            "db.diagUpToDate",
                            installed = installed,
                            latest = latest
                        ));
                    } else {
                        would_emit = Some(DiagWouldEmit {
                            installed_version: installed,
                            latest_version: latest,
                            source: best.source.clone(),
                        });
                    }
                }
            }
        }

        Ok(UpdateDiagnostic {
            folder_name: folder_name.to_string(),
            package,
            airport_match,
            catalog_entries,
            cache_entries,
            blocker,
            would_emit,
        })
    }

    /// (v5.2.0) Predicado SQL que excluye entradas del catálogo
    /// etiquetadas EXCLUSIVAMENTE para el otro simulador. Los escenarios
    /// y addons de MSFS 2024 NO son compatibles con 2020 y viceversa, así
    /// que ofrecer un 2024 como "update" de un paquete 2020 (mismo
    /// ICAO+dev) era incorrecto. La tabla `addons` es persistente y puede
    /// contener filas del otro sim (de una sesión previa, o de posts
    /// "MSFS 2024" cacheados), de ahí el filtro aquí y no solo en el scrape.
    ///
    /// Conservador a propósito: mantiene "MSFS 2020/2024" (compatible con
    /// ambos) y las filas SIN etiqueta (`simulator` vacío/NULL — p.ej.
    /// aviones de Simplaza). Solo descarta lo marcado únicamente para el
    /// sim contrario. Devuelve `&'static str` sin "AND" inicial.
    fn sim_catalog_filter() -> &'static str {
        if crate::sim::is_2024() {
            "NOT (COALESCE(a.simulator,'') LIKE '%2020%' AND COALESCE(a.simulator,'') NOT LIKE '%2024%')"
        } else {
            "NOT (COALESCE(a.simulator,'') LIKE '%2024%' AND COALESCE(a.simulator,'') NOT LIKE '%2020%')"
        }
    }

    pub async fn catalog_versions_for_community(
        pool: &SqlitePool,
    ) -> anyhow::Result<Vec<UpdateCandidate>> {
        // **SCENERY: ICAO + creator/developer match** — el match por
        // ICAO solo permitía falsos positivos como "Aerosoft EBBR
        // v1.0.5 → JustSim v1.1.0". Ahora exigimos que el creator
        // del manifest matchee por substring (case-insensitive) con
        // el developer del catálogo.
        let sim_filter = sim_catalog_filter();
        let scenery_sql = format!(
            "{}\n              AND {}",
            r#"
            SELECT cp.folder_name        AS folder_name,
                   cp.title              AS title,
                   UPPER(cp.icao)        AS icao,
                   cp.package_version    AS installed_version,
                   a.version             AS catalog_version,
                   a.source              AS catalog_source,
                   a.id                  AS catalog_addon_id,
                   a.page_url            AS catalog_page_url,
                   cp.creator            AS creator,
                   a.developer           AS developer
            FROM community_packages cp
            INNER JOIN addons a    ON UPPER(a.icao)  = UPPER(cp.icao)
            INNER JOIN airports ap ON ap.icao        = UPPER(cp.icao)
            WHERE cp.package_version IS NOT NULL AND cp.package_version <> ''
              AND a.version IS NOT NULL AND a.version <> ''
              AND UPPER(cp.content_type) = 'SCENERY'
              AND cp.creator IS NOT NULL AND cp.creator <> ''
              AND a.developer IS NOT NULL AND a.developer <> ''
              -- (v4.16.3) El match creator↔developer se hace en RUST
              -- (compute_available) normalizando a solo alfanuméricos —
              -- robusto ante guiones Unicode (catálogo "MK–STUDIOS" en-dash
              -- vs manifest "MK-STUDIOS" ASCII), que el INSTR de SQL no
              -- cazaba. Aquí solo exigimos que ambos campos no estén vacíos.
            "#,
            sim_filter
        );
        let scenery = sqlx::query_as::<_, UpdateCandidate>(&scenery_sql)
        .fetch_all(pool)
        .await?;

        // **AIRCRAFT/INSTRUMENT/MISC: creator + título** —
        // Simplaza distribuye sobre todo aviones; sus updates jamás
        // aparecían cuando el detector exigía ICAO + airports.
        // Match por: creator coincide Y nombre del catálogo es
        // substring del título (o viceversa).
        //
        // **Filtros anti-falsos-positivos** (en orden de impacto):
        //
        //   1. Asimetría livery/sound/etc — el bug que reportó el
        //      usuario: tener PMDG 737-600 (avión base) en Community
        //      hacía aparecer "Boeing 737-600 Liveries by PMDG" como
        //      update porque ambos tienen "737-600" en el nombre y
        //      "PMDG" como creator. Un avión base NUNCA puede ser
        //      "actualizado" por un livery pack — son artefactos
        //      distintos. La lista de tokens cubre los content-type
        //      tags que la comunidad usa por convención.
        //
        //   2. `LENGTH(cp.title) >= 6` — un título de 1-2 palabras
        //      genéricas ("Liveries", "Mods", "Pack") matchea con
        //      casi cualquier addon del mismo creator.
        //
        //   3. Lista negra de títulos triviales (case-insensitive).
        // Reglas para AIRCRAFT/INSTRUMENT/MISC (v0.1.9, mucho más
        // estrictas que v0.1.8):
        //
        //   1. Creator del manifest matchea con developer del catálogo
        //      (substring bidireccional).
        //   2. Hay un EXISTS en `installed_addons` que liga el folder
        //      a este addon_id concreto. Esto significa que el usuario
        //      DESCARGÓ este paquete via la app — no estamos
        //      adivinando vía heurística de título.
        //   3. Si NO hay link explícito, igual aceptamos pero con
        //      filtros agresivos: overlap de tokens >= 60% del
        //      nombre del catálogo, Y todos los anti-categoría
        //      (livery/sound/preset/etc) tienen que pasar.
        //
        // El usuario reportó como bug v0.1.8: tenía PMDG 737-600
        // base aircraft instalado y aparecía como "update" un
        // livery pack del catálogo. Aunque el filtro asimétrico
        // estaba (y debería catch eso), el caso real fallaba.
        // v0.1.9: cuando NO hay link explícito, EXIGIMOS overlap
        // de longitud + ABSENCE de palabras "categoría" en el
        // nombre del catálogo (no asimétrico — directamente
        // rechazamos cualquier livery/sound/preset/mod/paint/
        // texture/profile/config en a.name si el paquete instalado
        // no fue linkeado por el usuario).
        let aircraft_sql = format!(
            "{}\n              AND {}",
            r#"
            SELECT cp.folder_name        AS folder_name,
                   cp.title              AS title,
                   COALESCE(UPPER(cp.icao), '') AS icao,
                   cp.package_version    AS installed_version,
                   a.version             AS catalog_version,
                   a.source              AS catalog_source,
                   a.id                  AS catalog_addon_id,
                   a.page_url            AS catalog_page_url,
                   cp.creator            AS creator,
                   a.developer           AS developer
            FROM community_packages cp
            INNER JOIN addons a ON
              cp.creator IS NOT NULL AND cp.creator <> ''
              AND a.developer IS NOT NULL AND a.developer <> ''
              -- (v4.16.3) El match creator↔developer se hace en RUST
              -- (compute_available), robusto ante guiones Unicode. Aquí
              -- solo exigimos no-vacío + el match de modelo/título de abajo.
              AND (
                INSTR(LOWER(a.name), LOWER(cp.title)) > 0
                OR INSTR(LOWER(cp.title), LOWER(a.name)) > 0
                -- Shared aircraft-model token. SQLite GLOB es
                -- limitado pero suficiente para los modelos más
                -- comunes que actualizan: A3xx, A350, 737/738/739,
                -- 747, 777, etc. La idea: ambos lados contienen
                -- el mismo patrón. Ej. cp.title="iniBuilds A350-900"
                -- y a.name="Airbus A350" comparten "a350".
                OR (LOWER(cp.title) GLOB '*a350*' AND LOWER(a.name) GLOB '*a350*')
                OR (LOWER(cp.title) GLOB '*a330*' AND LOWER(a.name) GLOB '*a330*')
                OR (LOWER(cp.title) GLOB '*a320*' AND LOWER(a.name) GLOB '*a320*')
                OR (LOWER(cp.title) GLOB '*a319*' AND LOWER(a.name) GLOB '*a319*')
                OR (LOWER(cp.title) GLOB '*a321*' AND LOWER(a.name) GLOB '*a321*')
                OR (LOWER(cp.title) GLOB '*a380*' AND LOWER(a.name) GLOB '*a380*')
                OR (LOWER(cp.title) GLOB '*737-700*' AND LOWER(a.name) GLOB '*737-700*')
                OR (LOWER(cp.title) GLOB '*737-800*' AND LOWER(a.name) GLOB '*737-800*')
                OR (LOWER(cp.title) GLOB '*737-900*' AND LOWER(a.name) GLOB '*737-900*')
                OR (LOWER(cp.title) GLOB '*747-400*' AND LOWER(a.name) GLOB '*747-400*')
                OR (LOWER(cp.title) GLOB '*747-8*' AND LOWER(a.name) GLOB '*747-8*')
                OR (LOWER(cp.title) GLOB '*777-200*' AND LOWER(a.name) GLOB '*777-200*')
                OR (LOWER(cp.title) GLOB '*777-300*' AND LOWER(a.name) GLOB '*777-300*')
                OR (LOWER(cp.title) GLOB '*cessna*172*' AND LOWER(a.name) GLOB '*cessna*172*')
                OR (LOWER(cp.title) GLOB '*787-8*' AND LOWER(a.name) GLOB '*787-8*')
                OR (LOWER(cp.title) GLOB '*787-9*' AND LOWER(a.name) GLOB '*787-9*')
                OR (LOWER(cp.title) GLOB '*787-10*' AND LOWER(a.name) GLOB '*787-10*')
              )
            WHERE cp.package_version IS NOT NULL AND cp.package_version <> ''
              AND a.version IS NOT NULL AND a.version <> ''
              AND UPPER(cp.content_type) IN ('AIRCRAFT', 'INSTRUMENT', 'MISC')
              AND LENGTH(cp.title) >= 6
              AND LOWER(cp.title) NOT IN (
                'liveries', 'livery', 'sounds', 'sound pack', 'mods', 'mod',
                'pack', 'addon', 'tweak', 'fix', 'pro', 'premium', 'enhanced',
                'preset', 'presets', 'config', 'configs', 'profile', 'profiles'
              )
              AND (
                -- Caso A: el usuario instaló este addon vía nuestra
                -- app (fuerte señal — addon_id linkado).
                EXISTS (
                  SELECT 1 FROM installed_addons ia
                  WHERE ia.addon_id = a.id
                    AND (
                      ia.name = cp.folder_name
                      OR ia.install_path LIKE ('%' || cp.folder_name)
                      OR ia.install_path LIKE ('%' || cp.folder_name || '%')
                    )
                )
                OR
                -- Caso B: no hay link explícito, exigimos varios
                -- chequeos antes de creernos el match heurístico.
                -- El match puede venir por substring (variantes de
                -- nombre cortas) o por shared aircraft-model token
                -- (ej. "iniBuilds A350-900" ↔ "Airbus A350", que no
                -- comparten substring pero sí familia de avión).
                -- En ambos sub-casos exigimos AUSENCIA de palabras
                -- de categoría (livery/sound/etc) en a.name para
                -- evitar matches livery→aircraft.
                (
                  -- Sub-caso B.1: substring + token overlap >= 60%.
                  -- Evita matches "Boeing 737-600" (14 chars) con
                  -- "Boeing 737-600 Liveries by PMDG" (32 chars,
                  -- overlap 14/32 = 43%).
                  (
                    INSTR(LOWER(a.name), LOWER(cp.title)) > 0
                    AND LENGTH(cp.title) * 100 / NULLIF(LENGTH(a.name), 0) >= 60
                  )
                  OR (
                    INSTR(LOWER(cp.title), LOWER(a.name)) > 0
                    AND LENGTH(a.name) * 100 / NULLIF(LENGTH(cp.title), 0) >= 60
                  )
                  OR
                  -- Sub-caso B.2 (v0.1.18): shared aircraft-model
                  -- token. El JOIN ya aceptó por GLOB pero el WHERE
                  -- v0.1.17 rechazaba porque ninguno era substring
                  -- del otro. Bug reportado: "iniBuilds A350-900"
                  -- vs "Airbus A350" no aparecía como update.
                  (LOWER(cp.title) GLOB '*a350*' AND LOWER(a.name) GLOB '*a350*')
                  OR (LOWER(cp.title) GLOB '*a330*' AND LOWER(a.name) GLOB '*a330*')
                  OR (LOWER(cp.title) GLOB '*a320*' AND LOWER(a.name) GLOB '*a320*')
                  OR (LOWER(cp.title) GLOB '*a319*' AND LOWER(a.name) GLOB '*a319*')
                  OR (LOWER(cp.title) GLOB '*a321*' AND LOWER(a.name) GLOB '*a321*')
                  OR (LOWER(cp.title) GLOB '*a380*' AND LOWER(a.name) GLOB '*a380*')
                  OR (LOWER(cp.title) GLOB '*737-700*' AND LOWER(a.name) GLOB '*737-700*')
                  OR (LOWER(cp.title) GLOB '*737-800*' AND LOWER(a.name) GLOB '*737-800*')
                  OR (LOWER(cp.title) GLOB '*737-900*' AND LOWER(a.name) GLOB '*737-900*')
                  OR (LOWER(cp.title) GLOB '*747-400*' AND LOWER(a.name) GLOB '*747-400*')
                  OR (LOWER(cp.title) GLOB '*747-8*' AND LOWER(a.name) GLOB '*747-8*')
                  OR (LOWER(cp.title) GLOB '*777-200*' AND LOWER(a.name) GLOB '*777-200*')
                  OR (LOWER(cp.title) GLOB '*777-300*' AND LOWER(a.name) GLOB '*777-300*')
                  OR (LOWER(cp.title) GLOB '*cessna*172*' AND LOWER(a.name) GLOB '*cessna*172*')
                  OR (LOWER(cp.title) GLOB '*787-8*' AND LOWER(a.name) GLOB '*787-8*')
                  OR (LOWER(cp.title) GLOB '*787-9*' AND LOWER(a.name) GLOB '*787-9*')
                  OR (LOWER(cp.title) GLOB '*787-10*' AND LOWER(a.name) GLOB '*787-10*')
                )
                AND
                -- Absence de tokens categoría en el nombre del
                -- catálogo (livery/sound/preset/mod/paint/etc).
                -- Si el catálogo es eso, NO aplica este caso B.
                NOT (
                  LOWER(a.name) LIKE '%liveries%'
                  OR LOWER(a.name) LIKE '%livery%'
                  OR LOWER(a.name) LIKE '%paint%'
                  OR LOWER(a.name) LIKE '%texture%'
                  OR LOWER(a.name) LIKE '%sounds%'
                  OR LOWER(a.name) LIKE '%sound pack%'
                  OR LOWER(a.name) LIKE '%soundpack%'
                  OR LOWER(a.name) LIKE '%preset%'
                  OR LOWER(a.name) LIKE '%profile%'
                  OR LOWER(a.name) LIKE '%config%'
                  OR LOWER(a.name) LIKE '%mod %'
                  OR LOWER(a.name) LIKE '% mod'
                  OR LOWER(a.name) LIKE '%tweak%'
                  OR LOWER(a.name) LIKE '%enhancement%'
                  OR LOWER(a.name) LIKE '%pack%'
                )
              )
            "#,
            sim_filter
        );
        let aircraft = sqlx::query_as::<_, UpdateCandidate>(&aircraft_sql)
        .fetch_all(pool)
        .await?;

        let mut all = scenery;
        all.extend(aircraft);
        Ok(all)
    }

    // ─────────── download_jobs (v0.1.21 — resume tras cierre) ───────────

    /// Persiste el estado actual de un job — usado tanto al crear
    /// como al actualizar. INSERT OR REPLACE para que cada llamada
    /// sea idempotente y simple.
    ///
    /// `created_at` se respeta en updates (no se sobreescribe con
    /// `now`) — el caller pasa el valor original del job.
    pub async fn upsert_download_job(
        pool: &SqlitePool,
        job: &DownloadJob,
    ) -> anyhow::Result<()> {
        let phase = phase_to_str(&job.phase);
        sqlx::query(
            r#"
            INSERT INTO download_jobs (
                id, addon_id, addon_title, source, method_kind, method_name,
                url, phase, bytes_total, bytes_done, speed_bps, eta_seconds,
                message, error, install_path, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, datetime('now')
            )
            ON CONFLICT(id) DO UPDATE SET
                addon_id     = excluded.addon_id,
                addon_title  = excluded.addon_title,
                source       = excluded.source,
                method_kind  = excluded.method_kind,
                method_name  = excluded.method_name,
                url          = excluded.url,
                phase        = excluded.phase,
                bytes_total  = excluded.bytes_total,
                bytes_done   = excluded.bytes_done,
                speed_bps    = excluded.speed_bps,
                eta_seconds  = excluded.eta_seconds,
                message      = excluded.message,
                error        = excluded.error,
                install_path = excluded.install_path,
                updated_at   = datetime('now')
            "#,
        )
        .bind(&job.id)
        .bind(&job.addon_id)
        .bind(&job.addon_title)
        .bind(&job.source)
        .bind(&job.method_kind)
        .bind(&job.method_name)
        .bind(&job.url)
        .bind(phase)
        .bind(job.bytes_total as i64)
        .bind(job.bytes_done as i64)
        .bind(job.speed_bps as i64)
        .bind(job.eta_seconds.map(|n| n as i64))
        .bind(&job.message)
        .bind(&job.error)
        .bind(&job.install_path)
        .bind(&job.created_at)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Borra el job de DB. Lo invoca el manager cuando el usuario
    /// pulsa "✕" sobre una entrada del panel de descargas.
    pub async fn delete_download_job(pool: &SqlitePool, id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM download_jobs WHERE id = ?1")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Lee todos los jobs persistidos. Orden estable por `created_at`
    /// para que la UI los pinte como los dejó el usuario antes de
    /// cerrar. Filas con phases desconocidas se ignoran (defensa
    /// contra futuros cambios de enum).
    pub async fn list_download_jobs(pool: &SqlitePool) -> anyhow::Result<Vec<DownloadJob>> {
        let rows: Vec<DownloadJobRow> = sqlx::query_as(
            r#"
            SELECT id, addon_id, addon_title, source, method_kind, method_name,
                   url, phase, bytes_total, bytes_done, speed_bps, eta_seconds,
                   message, error, install_path, created_at
            FROM download_jobs
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let Some(phase) = phase_from_str(&r.phase) else {
                tracing::warn!("download_jobs: phase desconocida '{}' en id={}", r.phase, r.id);
                continue;
            };
            out.push(DownloadJob {
                id: r.id,
                addon_id: r.addon_id,
                addon_title: r.addon_title,
                source: r.source,
                method_kind: r.method_kind,
                method_name: r.method_name,
                url: r.url,
                phase,
                bytes_total: r.bytes_total as u64,
                bytes_done: r.bytes_done as u64,
                speed_bps: r.speed_bps as u64,
                eta_seconds: r.eta_seconds.map(|n| n as u64),
                message: r.message,
                error: r.error,
                install_path: r.install_path,
                created_at: r.created_at,
            });
        }
        Ok(out)
    }

    #[derive(sqlx::FromRow)]
    struct DownloadJobRow {
        id: String,
        addon_id: String,
        addon_title: String,
        source: String,
        method_kind: String,
        method_name: String,
        url: String,
        phase: String,
        bytes_total: i64,
        bytes_done: i64,
        speed_bps: i64,
        eta_seconds: Option<i64>,
        message: Option<String>,
        error: Option<String>,
        install_path: Option<String>,
        created_at: String,
    }

    fn phase_to_str(p: &crate::download::DownloadPhase) -> &'static str {
        use crate::download::DownloadPhase::*;
        match p {
            Queued      => "queued",
            Resolving   => "resolving",
            Downloading => "downloading",
            Paused      => "paused",
            Installing  => "installing",
            Completed   => "completed",
            Cancelled   => "cancelled",
            Error       => "error",
        }
    }

    fn phase_from_str(s: &str) -> Option<crate::download::DownloadPhase> {
        use crate::download::DownloadPhase::*;
        Some(match s {
            "queued"      => Queued,
            "resolving"   => Resolving,
            "downloading" => Downloading,
            "paused"      => Paused,
            "installing"  => Installing,
            "completed"   => Completed,
            "cancelled"   => Cancelled,
            "error"       => Error,
            _ => return None,
        })
    }
}
