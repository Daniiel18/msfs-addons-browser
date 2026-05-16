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
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

pub mod repo {
    use sqlx::SqlitePool;

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
        pub airport_name: Option<String>,
        pub latitude: Option<f64>,
        pub longitude: Option<f64>,
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
        Ok(rows)
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
    /// hasta que el usuario pulse "Recargar" o la instale.
    pub async fn dismiss_update(pool: &SqlitePool, folder_name: &str) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO dismissed_updates (folder_name, dismissed_at)
            VALUES (?1, datetime('now'))
            ON CONFLICT(folder_name) DO UPDATE SET dismissed_at = excluded.dismissed_at
            "#,
        )
        .bind(folder_name)
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

    /// Devuelve el set de folder_names dismissed. Lo cargamos una
    /// vez en `compute_available` y filtramos en memoria — más
    /// barato que JOIN cuando hay sólo decenas de filas.
    pub async fn list_dismissed_folder_names(
        pool: &SqlitePool,
    ) -> anyhow::Result<std::collections::HashSet<String>> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT folder_name FROM dismissed_updates")
            .fetch_all(pool)
            .await?;
        Ok(rows.into_iter().map(|(s,)| s).collect())
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
            blocker = Some(format!(
                "El paquete '{}' no está en community_packages — re-escanea Community.",
                folder_name
            ));
        } else if let Some(pkg) = package.as_ref() {
            if pkg.icao.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true) {
                blocker = Some(
                    "El paquete no tiene ICAO extraído. Sin ICAO no se puede cruzar con el catálogo. Causas típicas: el manifest no tiene 'SCENERY' como content_type, o el título/folder no contiene un código ICAO de 4 letras."
                        .to_string(),
                );
            } else if pkg
                .content_type
                .as_deref()
                .map(|s| !s.eq_ignore_ascii_case("SCENERY"))
                .unwrap_or(true)
            {
                blocker = Some(format!(
                    "content_type es '{}' — la detección de updates exige 'SCENERY'. Si crees que es scenery, edita el manifest.",
                    pkg.content_type.as_deref().unwrap_or("(vacío)")
                ));
            } else if pkg
                .package_version
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                blocker = Some(
                    "El manifest no declara 'package_version'. Sin versión instalada no se puede comparar."
                        .to_string(),
                );
            } else if airport_match.is_none() {
                blocker = Some(format!(
                    "El ICAO '{}' no existe en la tabla 'airports' (dataset OurAirports). Sin esto se descartan falsos positivos. Si el ICAO es real, dispara 'refresh airports dataset' o verifica que se hayan descargado los datos.",
                    icao_upper.as_deref().unwrap_or("?")
                ));
            } else {
                let with_version: Vec<&DiagCatalog> = catalog_entries
                    .iter()
                    .filter(|e| e.version.as_ref().map(|v| !v.trim().is_empty()).unwrap_or(false))
                    .collect();

                if with_version.is_empty() {
                    blocker = Some(format!(
                        "El catálogo (addons) no tiene ninguna entrada con versión para ICAO '{}'. Causas: la búsqueda en SceneryAddons/Simplaza no devolvió nada con ese ICAO, o el parser no extrajo la versión del título. Pulsa 'Refresh updates' o busca el ICAO manualmente para alimentar la cache.",
                        icao_upper.as_deref().unwrap_or("?")
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
                        blocker = Some(format!(
                            "Versión instalada '{}' >= mejor versión catalogada '{}' — no hay update real.",
                            installed, latest
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

    pub async fn catalog_versions_for_community(
        pool: &SqlitePool,
    ) -> anyhow::Result<Vec<UpdateCandidate>> {
        // **SCENERY: ICAO + creator/developer match** — el match por
        // ICAO solo permitía falsos positivos como "Aerosoft EBBR
        // v1.0.5 → JustSim v1.1.0". Ahora exigimos que el creator
        // del manifest matchee por substring (case-insensitive) con
        // el developer del catálogo.
        let scenery = sqlx::query_as::<_, UpdateCandidate>(
            r#"
            SELECT cp.folder_name        AS folder_name,
                   cp.title              AS title,
                   UPPER(cp.icao)        AS icao,
                   cp.package_version    AS installed_version,
                   a.version             AS catalog_version,
                   a.source              AS catalog_source,
                   a.id                  AS catalog_addon_id,
                   a.page_url            AS catalog_page_url
            FROM community_packages cp
            INNER JOIN addons a    ON UPPER(a.icao)  = UPPER(cp.icao)
            INNER JOIN airports ap ON ap.icao        = UPPER(cp.icao)
            WHERE cp.package_version IS NOT NULL AND cp.package_version <> ''
              AND a.version IS NOT NULL AND a.version <> ''
              AND UPPER(cp.content_type) = 'SCENERY'
              AND cp.creator IS NOT NULL AND cp.creator <> ''
              AND a.developer IS NOT NULL AND a.developer <> ''
              AND (
                INSTR(LOWER(cp.creator), LOWER(a.developer)) > 0
                OR INSTR(LOWER(a.developer), LOWER(cp.creator)) > 0
              )
            "#,
        )
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
        let aircraft = sqlx::query_as::<_, UpdateCandidate>(
            r#"
            SELECT cp.folder_name        AS folder_name,
                   cp.title              AS title,
                   COALESCE(UPPER(cp.icao), '') AS icao,
                   cp.package_version    AS installed_version,
                   a.version             AS catalog_version,
                   a.source              AS catalog_source,
                   a.id                  AS catalog_addon_id,
                   a.page_url            AS catalog_page_url
            FROM community_packages cp
            INNER JOIN addons a ON
              cp.creator IS NOT NULL AND cp.creator <> ''
              AND a.developer IS NOT NULL AND a.developer <> ''
              AND (
                INSTR(LOWER(cp.creator), LOWER(a.developer)) > 0
                OR INSTR(LOWER(a.developer), LOWER(cp.creator)) > 0
              )
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
                -- Caso B: no hay link, exigimos varios chequeos
                -- antes de creernos el match heurístico.
                (
                  -- Token overlap: cp.title debe ser >= 60% del
                  -- largo de a.name (o viceversa). Evita matches
                  -- "Boeing 737-600" (14 chars) con
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
        )
        .fetch_all(pool)
        .await?;

        let mut all = scenery;
        all.extend(aircraft);
        Ok(all)
    }
}
