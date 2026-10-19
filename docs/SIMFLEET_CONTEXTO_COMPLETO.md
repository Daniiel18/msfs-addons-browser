# SimFleet (msfs-addons-browser) — Contexto completo para análisis

> **Para la IA que lee esto:** este documento describe la aplicación SimFleet entera —producto, interfaz, backend, datos, integraciones, build, entorno de pruebas, historial reciente y problemas conocidos— para que puedas analizarla sin tener el código delante. La **Parte I** es narrativa (cómo funciona y por qué). La **Parte II** son inventarios **generados automáticamente desde el código** el 2026-10-18 (todos los comandos, tablas, componentes, módulos, eventos, claves, URLs y dependencias), así que no falta ninguna pieza. Si algo de la Parte I contradice a la Parte II, manda la Parte II. Las rutas son relativas a la raíz del repo. Los comentarios del código están en español y suelen llevar etiquetas de versión tipo `(v6.2.38)`, que indican cuándo se añadió algo.

- Repo: `github.com/Daniiel18/msfs-addons-browser` (público, licencia propietaria "All Rights Reserved").
- Versión publicada: **7.4.1**. En el árbol de trabajo hay cambios **sin commitear** marcados `(v7.4.2)` (ver §12).
- Tamaño: ~145.000 líneas (≈ 52 % Rust, 48 % TypeScript/TSX + JSON de i18n).

---

# PARTE I — Narrativa

## 1. Qué es SimFleet

App de escritorio **Windows 10/11** compañera de **Microsoft Flight Simulator 2020 y 2024**. Junta en una sola ventana dos mundos:

1. **Addons**: buscar/navegar catálogos (SceneryAddons, Simplaza; Skybound desactivado), descargar (cliente BitTorrent embebido + navegador embebido para hosters tipo mixdrop/modsfire) e **instalar** directo en la carpeta Community correcta (2020 o 2024). Gestiona lo instalado: escaneo de Community, clasificación, activar/desactivar, desinstalar, actualizaciones, liveries PMDG/iFly/flightsim.to, perfiles GSX, rendimiento (FPS) por aeropuerto, mapa de escenarios, Health Center.
2. **Vuelo**: seguimiento en vivo vía **SimConnect** (logbook automático con fases OOOI, track GPS, aterrizaje y FPM, METAR, daños), **scoring** estilo aerolínea virtual, **FlightBook** con mapa 3D, **Hangar** (estadísticas por avión, mantenimiento, Best Landings en vídeo), **economía de aerolínea** (Finanzas), **logros**, integración con **SimBrief**, **GSX Pro**, **VAS-ACARS**, **Navigraph/AIRAC**, **Discord Rich Presence**, **Live VS** (duelo de aterrizajes en tiempo real entre dos pilotos vía Supabase) y **sincronización en Google Drive**.

Usuarios reales: el dueño (**Daniel**, Rep. Dominicana, UTC-4, vuela sobre todo **MSFS 2024** con Fenix A319/A320/A321, PMDG, iFly 737 MAX, aerolíneas como Delta/WestJet) y un amigo (**Héctor**). Varias funciones (Live VS, identidades "daniel"/"hector", allowlist de cuentas de Drive por hash en `owner_ids.rs`) están pensadas para esos dos pilotos. Idiomas de UI: español e inglés.

Hay además un **demo web público** (GitHub Pages, `daniiel18.github.io/msfs-addons-browser/`) que ejecuta solo la UI con datos falsos (`demoApi`).

## 2. Arquitectura

```
┌──────────────────────────── Proceso simfleet.exe (Tauri v2) ───────────────────────────┐
│                                                                                         │
│  WebView2 "main"  ── React 18 + TS + Vite + Tailwind + Zustand + Framer Motion          │
│   (UI completa)      MapLibre GL 5 · Recharts · @xyflow/react · SunCalc                 │
│        │  invoke("comando") / listen("evento")      (src/lib/tauri.ts = capa Api)       │
│        ▼                                                                                │
│  Backend Rust (src-tauri/src) — 152 #[tauri::command] + tareas tokio en segundo plano  │
│   ├─ AppState (lib.rs): pool SQLite (sqlx), fuentes, DownloadManager, reqwest, …        │
│   ├─ simconnect_watcher (FFI a SimConnect.dll vía libloading) ─▶ flight_log/track       │
│   ├─ download::manager (librqbit torrent) · install (unrar/sevenz/zip) · drop_install   │
│   ├─ cloud_sync (Google Drive appDataFolder) · gsx · simbrief · vas_acars · scoring …   │
│   └─ landing_recorder (Windows Graphics Capture + H.264 por hardware + WASAPI)          │
│                                                                                         │
│  WebView2 "osd"   ── overlay transparente siempre-encima (aterrizaje FPM)               │
│  WebView2 "livery-browser" / "hoster-*" ── navegadores embebidos de descarga            │
│        (flightsim.to con la cuenta del usuario; mixdrop/modsfire/get.php)               │
└─────────────────────────────────────────────────────────────────────────────────────────┘
        │ SQLite: <data>/msfs-addons.db        │ Logs: <data>/logs/simfleet.YYYY-MM-DD.log
        ▼
   data dir PORTABLE: `<carpeta del exe>/data/` (si no es escribible → %LOCALAPPDATA%\SimFleet\data)
```

- **Tauri v2** (`tauri 2.10`, wry 0.54) con plugins: shell, opener, dialog, autostart, single-instance (salvo en pruebas), notification. Config en `src-tauri/tauri.conf.json`: `identifier = org.n0xful.msfsaddonsbrowser`, `productName = SimFleet`, binario `simfleet`, ventanas `main` (sin decoraciones, titlebar propia `TitleBar.tsx`, arranca 460×540 como splash y se agranda) y `osd` (transparente, alwaysOnTop, oculta). CSP desactivada. `assetProtocol` habilitado para `$VIDEO`, `$APPDATA`, `$HOME`, etc. Recursos empaquetados: `airports.csv`, `SimConnect.dll`, `ublock-lite.zip`.
- **Capabilities** (`src-tauri/capabilities/default.json`, `osd.json`): permisos de ventana, opener, dialog, autostart, notification. **No** incluye `core:window:allow-set-title` (por eso el título de pruebas se pone desde Rust).
- **Estado**: `AppState` en `src-tauri/src/lib.rs` (pool SQLite, `Vec<Arc<dyn Source>>`, `DownloadManager`, `GsxClient`, `reqwest::Client`, flags `minimize_to_tray`, sesiones de drag&drop, auth de Skybound, mapa de descargas de liveries, `data_dir`). El watcher de SimConnect se gestiona como estado aparte (`SharedState`).
- **Comunicación**: frontend → backend con `invoke`; backend → frontend con eventos (`app.emit`), p.ej. `flight://current`, `download://update`, `cloud://progress`, `score:done` (tabla completa en Apéndice C).
- **SQLite** vía `sqlx 0.8` con migraciones embebidas (`sqlx::migrate!("./migrations")`, 63 archivos). Tabla k/v `settings` para casi toda la configuración.
- **Logs**: `tracing` + `tracing-appender` a `<data>/logs/simfleet.<fecha>.log` (formato `[YYYY/MM/DD HH:MM:SS] [NIVEL] [target] mensaje`), más los informes por vuelo en `<data>/logs/flights/`. `cmd_log!`/`CmdTimer` registran cada comando y marcan "LENTO" los que tardan.
- **TEMP redirigido**: al arrancar, `TMP/TEMP` del proceso apuntan a `<data>/temp` (para que extracciones y drag&drop no ensucien `%TEMP%`).
- **Single instance + bandeja**: abrir de nuevo trae la ventana existente; "minimizar a la bandeja" oculta en vez de cerrar.

## 3. Estructura del repositorio

```
/                         package.json (scripts dev/build/tauri), vite.config.ts, tailwind/postcss,
                          tsconfig*.json, index.html, README/CHANGELOG/CONTRIBUTING/SECURITY/LICENSE,
                          dev-safe.ps1 (lanza el ENTORNO DE PRUEBAS), .github/ (workflows + plantillas)
src/                      Frontend React
  main.tsx, App.tsx       Arranque, splash de 9 pasos, navegación por vistas, banners, modales globales
  components/             66 componentes (vistas, modales, widgets) — Apéndice E
  stores/                 20 stores Zustand (estado global) — Apéndice F
  lib/                    36 módulos: tauri.ts (capa Api), types.ts, i18n, utilidades — Apéndice G
  styles/globals.css      Tailwind + estilos globales
public/                   aircraft/ (siluetas), whatsnew/ (SVG de novedades)
src-tauri/                Backend Rust
  src/lib.rs              run(): plugins, AppState, init_state(), tareas de fondo, generate_handler!
  src/commands/           Capa de comandos Tauri (fina; delega en los módulos de dominio)
  src/*.rs                Módulos de dominio (watcher, cloud, gsx, scoring, economía…) — Apéndice D
  src/sources/            sceneryaddons.rs, simplaza.rs, skybound.rs (trait Source)
  src/download/           manager.rs (cola de descargas), torrent.rs (librqbit)
  src/install/            extracción RAR/7z/ZIP e instalación
  src/scoring/            mod.rs + rubric.rs (reglas por fase)
  src/db/                 init + repo (consultas compartidas)
  migrations/             001…063 *.sql
  resources/              airports.csv, ublock-lite.zip, SimConnect.dll
  build.rs                inyecta secrets.local.toml (credenciales OAuth Google) como env de compilación
docs/                     SQL de Supabase (vs_results, tablas _test)
scripts/                  build-release.ps1 (build limpio del instalador NSIS), scripts de i18n (.py)
.notes/                   landingtoast_analysis.md (análisis de la app LandingToast que inspiró el OSD/recorder)
```

## 4. Arranque

**Backend** (`lib.rs::run` → `setup` → `init_state`):
1. Detecta **entorno de pruebas** (`test_env::is_active()`; ver §11). En pruebas: fija `WEBVIEW2_USER_DATA_FOLDER`, se salta single-instance, autoarranque con nombre propio, inyecta `window.__SIMFLEET_TEST_ENV__`.
2. Resuelve el **data dir** (`resolve_portable_data_dir`), migra datos legacy de `%APPDATA%\org.n0xful.msfsaddonsbrowser` si hace falta, redirige TEMP, purga temporales de descargas embebidas, extrae uBlock Origin Lite, inicia el logger.
3. Limpieza del rebrand ("MSFS Addons Browser" → SimFleet), detección de la carpeta Community (lee `UserCfg.opt` de MSFS 2020/2024 Steam/MS Store), cliente HTTP, fuentes, **DB + migraciones**, credenciales de Skybound, `DownloadManager` (restaura jobs persistidos), watcher de SimConnect, controlador de Best Landings, Discord RPC, **auto-sync diario con Drive** (15 s tras arrancar, si pasaron >24 h), flag de bandeja, icono de bandeja.

**Frontend** (`App.tsx`): splash con 9 tareas visibles (`wrap(i, …)`): comprobar update de la app (puede ofrecer instalar), cargar fuentes, settings, SimBrief, Flight Log, suscribirse a descargas, **escanear Community**, comprobar updates de addons, precargar catálogos. Luego la ventana crece a tamaño completo. Modales de primer arranque: elección de versión de MSFS (si hay ambas), tour de onboarding, "What's New" anti-skip (carrusel por versión).

## 5. Frontend (UI)

### 5.1 Vistas principales (pestañas)
| Vista | Componente | Qué hace |
|---|---|---|
| Dashboard | `DashboardView.tsx` | "Ready to fly?" (pre-vuelo), avisos (update de GSX, AIRAC, Health Center), KPIs (paquetes, **Addons en disco**, aeropuertos, updates), desglose por tipo, top desarrolladores, más pesados, Wrapped anual compartible |
| Search | `App.tsx` + `ResultCard.tsx` | Pestañas por fuente (SceneryAddons, Simplaza; Skybound oculto), búsqueda y "browse" paginado, tarjetas con métodos de descarga priorizados |
| Map (scenery) | `MapView.tsx`, `MapAirportCard.tsx` | Mapa MapLibre de los aeropuertos/escenarios instalados (filtros GSX/updates). Fondo Esri Dark Gray (oscuro) u OSM (claro) |
| Addons | `AddonsView.tsx`, `PackageDetailModal.tsx` | Inventario de Community: tipos, activar/desactivar, desinstalar, reparar, updates de flightsim.to, botón "Get Liveries" (menú por modelo si el paquete trae varios), `LinkMapView.tsx` (grafo de dependencias con xyflow + dagre) |
| FlightBook | `FlightBookView.tsx`, `RoutesMapView.tsx` | Logbook: lista filtrable, detalle del vuelo, globo 3D satelital (Esri World Imagery + nombres Esri) con rutas, track real, terminator día/noche (4 bandas de crepúsculo), replay del track, live flight, Live VS, checklist de scoring, METAR/NOTAM, daños, compartir tarjeta PNG, editar/borrar/importar |
| Hangar | `HangarView.tsx`, `AircraftMaintenance.tsx` | Por avión/flota: estadísticas, mantenimiento (desgaste por zonas, alertas), Best Landings (clips), logros |
| Finance | `EconomyView.tsx` | Economía de aerolínea derivada de los vuelos (P&L, flota, políticas, carga), recibos reales de GSX (combustible, catering, handling) |

Otros elementos globales: `TitleBar`, `UpdateBanner`/`LiveUpdateBadge`/`UpdateWizard` (updater), `NotificationsBell` (centro de avisos), `DownloadsPanel`, `CommandPalette` (Ctrl+K), `DragDropOverlay` + `DropSelectModal` (instalar arrastrando archivos), `SplashScreen`, `OnboardingTour`, `WhatsNewModal`, `PreflightModal`, `SettingsModal` (4 grupos: General [general, tour, about, diagnostics] · Vuelo [flights, map_display, discord, gsx] · Datos [cloud, msfs_logbook, import, export] · Almacenamiento [folders, backup, storage]), `DiagnosticsSettings` (árbol de procesos, actividad de subsistemas), `ReplayBanner` (replay/slew en el sim), `FlyingNowBadge`, `PilotBadge` (avatar), `LiveVsManager` (headless), `ShareCardModal`.

### 5.2 Capa de datos del frontend
- **`src/lib/tauri.ts`**: interfaz `Api` (≈170 métodos) con dos implementaciones: `realApi` (`invoke`/`listen` de Tauri) y `demoApi` (datos falsos para el demo web). `isTauri` decide. `checkForUpdate` comparte la misma promesa 60 s (había 4 componentes consultando GitHub).
- **Stores Zustand** (`src/stores/`): settings, community, downloads, flight log, live VS, AIRAC, GSX updates, flightsim.to updates, toasts, etc. (Apéndice F).
- **Tipos** en `src/lib/types.ts` (espejo de los structs serde del backend, `camelCase`).
- **i18n**: `src/lib/i18n.ts` + `es.json`/`en.json` (1924 claves cada uno, sin faltantes). El backend tiene su propio diccionario (`i18n.rs`, macro `tr!`) y el frontend le empuja el idioma.
- **localStorage** (estado por máquina, no sincronizado): ver §7.3.
- **Mapas**: MapLibre GL 5. Proveedores sin clave: Esri World Imagery + Reference/World_Boundaries_and_Places (FlightBook), Esri Canvas World_Dark_Gray_Base/Reference (mapa oscuro y mini-mapa), OSM `tile.openstreetmap.org` (tema claro). Glyphs: `demotiles.maplibre.org`. Terminator calculado en `src/lib/terminator.ts` (geometría exacta por meridiano, refresco 30 s y al volver a la ventana).
- **Tarjetas compartibles** (`src/lib/shareCards.ts` + `shareImage.ts`): SVG autocontenido → PNG en canvas (fondo opaco) → portapapeles o archivo.

## 6. Backend por subsistema

### 6.1 Fuentes de addons y búsqueda
- Trait `Source` (`src-tauri/src/sources/mod.rs`): `search`, `browse(page)`, `descriptor`. Tipos `Addon`, `DownloadMethod {kind: Torrent|Mirror|Direct, url, parts[], password}`. `prioritize_methods` ordena torrent → modsfire → mixdrop → resto y **oculta rapidgator** (su Cloudflare bloquea la descarga embebida).
- `SceneryAddonsSource` (WordPress, filtra por "MSFS 2020/2024" en el título según el sim elegido), `SimplazaSource` (WordPress; categoriza casi todo como msfs-2020; en modo 2024 se muestra todo el catálogo y se etiqueta por título), `SkyboundSource` (API JSON con login; **desactivada** con `SKYBOUND_ENABLED = false` en `lib.rs` porque el sitio devuelve HTTP 530 desde el 10/10/2026).
- `SourceError::{Http, Parse, NotImplemented, Other, Unavailable}`; `is_unavailable()` detecta fuentes caídas.
- **Detector de updates** (`updates.rs`): cruza ICAOs y keywords de lo instalado con las fuentes, cachea por (término, fuente), "cortacircuitos" que deja de consultar una fuente caída durante el escaneo; `compute_available` agrupa por carpeta y versión.

### 6.2 Descargas
- **Cola** (`download/manager.rs`): jobs persistidos en DB, eventos `download://update`, pausa/reanudación/cancelación, auto-instalación al terminar, recuperación automática torrent → mirror cuando un torrent no tiene seeds.
- **Torrent** (`download/torrent.rs`): sesión única de **librqbit 8** (TLS nativo schannel), progreso y velocidad, timeout de metadata.
- **Navegadores embebidos** (`commands/livery.rs`, 1800 líneas):
  - *Liveries de flightsim.to* (`open_livery_browser`): ventana WebView2 donde el usuario usa su cuenta; captura la descarga, acelera la cuenta atrás de flightsim.to (pisa `setTimeout`/`setInterval` en rango 500–2000 ms tras `load`), asocia la descarga al `file_id` para seguir updates, vigila la carpeta Descargas del usuario.
  - *Hosters* (`open_hoster_download`): mixdrop, modsfire, `get.php` de SceneryAddons; overlay propio (bloqueo durante la descarga, banner "resuelve el captcha", botón Cancelar), auto-avance del botón de modsfire, archivos multi-parte (descarga en orden y unión), contraseña del archivo, progreso real desde el evento nativo `DownloadStarting`.
  - **Guardas** (`install_web_guards`): bloqueador de anuncios a nivel de red (97 dominios + endpoint de ads de flightsim.to, respuesta 403) **solo filtrando esos dominios**; bloqueo de popups hacia dominios de anuncios; F5/menú contextual desactivados en hosters. **uBlock Origin Lite** cargado como extensión de WebView2.
  - Los scripts inyectados van envueltos en `top_frame_only` (ver §12): wry en Windows los mete en **todos** los frames y eso rompía Cloudflare Turnstile.
  - El cliente `reqwest` que reanuda la descarga final usa un UA de Chrome fijo (`CHROME_UA`); las ventanas WebView ya **no** falsifican el UA.

### 6.3 Instalación y Community
- `install/mod.rs`: extrae RAR (multi-volumen, `unrar`), 7z (`sevenz-rust2`, AES) y ZIP (AES) e instala en Community; comprobación de espacio; contraseñas conocidas.
- `drop_install.rs`: drag&drop universal (perfiles GSX `.ini/.py`, paquetes Community, mixtos), flujo inspect → commit → cleanup con sesiones temporales; detección de compatibilidad 2020/2024; liveries PMDG/iFly.
- `community.rs`: detecta carpetas Community por `UserCfg.opt` (2020/2024, Steam/MS Store), raíces de trabajo de addons (PMDG work folder, etc.).
- `community_scanner.rs`: escanea cada paquete (`manifest.json`, `layout.json`, SimObjects, si tiene modelo/flight model, ICAO de escenarios por BGL/carpeta, miniaturas) → tabla `community_packages`. `has_flight_model` decide si es avión real (detecta PMDG/Fenix cifrados).
- `package_ops.rs` (activar/desactivar/desinstalar/reparar), `cross_link.rs` (junctions NTFS para usar un paquete en la Community de la otra versión), Health Center (`commands/community.rs`: addons 2024 en Community 2020, liveries huérfanas, restos), `pmdg_liveries.rs`/`pmdg_install.rs`, `livery_meta.rs`, `flightsim_track.rs` (seguimiento de updates de archivos de flightsim.to).
- `perf_config.rs` + `commands/perf.rs`: "motor de rendimiento" por aeropuerto (toggles de objetos/texturas para ganar FPS; aplicar/restaurar).
- `airports.rs` (dataset OurAirports en `resources/airports.csv`, mapa), `airport_thumbs.rs` (imágenes de Wikipedia para aeropuertos sin miniatura), `planespotters.rs` (fotos de aviones por matrícula).

### 6.4 Seguimiento de vuelo (SimConnect)
- `simconnect_ffi.rs`: bindings manuales a `SimConnect.dll` con `libloading` (sin bindgen). `simconnect_watcher.rs` (5.600 líneas) es la máquina de estados:
  - Suscribe ~70 simvars (posición, velocidades, actitud, motores ×4, luces, flaps, tren, frenos, combustible, transpondedor…), metadatos del avión (TITLE/ATC TYPE/AIRLINE/ID cada 1 s), stream de touchdown a frecuencia de frame para el FPM exacto, evento Pause, facility data AIRPORT/TAXI_PARKING (gate), puente LVar vía MobiFlight WASM (temperaturas de frenos, gate de GSX).
  - **Fases**: preflight → engine_running → pushback → taxi_out → takeoff → climbing → cruise → descent → approach → landed_rollout → taxi_in → parking → deboarding (OOOI). Detecta **replay/slew** (congela la máquina), reanudaciones tras crash del sim ("restore huérfano"), cancela block-outs que nunca despegan.
  - Al despegar `flight_log::start_flight`; al aterrizar `finish_flight` (origen/destino por proximidad a `airports`, distancia, FPM, METAR de SimBrief, gate GSX). Emite `flight://current`, OSD de aterrizaje, alimenta Live VS.
  - Fallback sin SimConnect: detecta el proceso `FlightSimulator(2024).exe` y cruza con el último OFP de SimBrief.
- `flight_log.rs`: persistencia (`flight_log`, `flight_log_track`, `flight_log_phase`), grados de aterrizaje, cierre de vuelos huérfanos. `damage.rs`: veredicto de daños/forzado. `flight_report.rs`: informe por vuelo a disco.
- **Importadores**: `vas_acars.rs` + `vas_summary.rs` (VAS-ACARS: lee sus `.bin` de track y resúmenes), `msfs_logbook.rs` (logbook nativo de MSFS). Deduplicación por `(source, external_id)` y por clave estable `started_at|lat|lon`; índice único `uq_flight_log_va_dedup_completed` (1 vuelo *completed* por aerolínea + nº de vuelo + día).

### 6.5 Scoring, informes y progresión
- `scoring/mod.rs` + `scoring/rubric.rs` (2.000 líneas de reglas): puntúa cada vuelo por fases (rubric con `points_max`, `ScoreItem` con evidencia JSON), guarda total/grado en `flight_log` y el desglose en `flight_log_score_item`. Se ejecuta al terminar cada vuelo y dispara el auto-upload a Drive (solo si hay sesión).
- `achievements.rs`: 23 medallas por niveles (bronce → diamante) calculadas del historial.
- `aircraft_maintenance.rs`: desgaste por zonas/ciclos, alertas, log de mantenimiento (Hangar y Finanzas comparten la fuente).
- `airline_economy.rs` (1.750 líneas): valor base realista por aerolínea, P&L por vuelo (ingresos pax/carga, combustible, tasas, catering, handling con recibos reales de GSX), flota, políticas, métricas.

### 6.6 Integraciones
- **SimBrief** (`simbrief.rs`, `commands/simbrief.rs`): pilot ID, OFP vigente (polling cada 30 s mientras hay sim), lista de vuelos, vincular OFP a un vuelo (auto-link al confirmar; si choca con el índice de dedup enlaza sin nº de vuelo), fixes de la ruta para el mapa.
- **GSX Pro** (`gsx.rs`, `gsx_update.rs`, `gsx_menu.rs`, `gsx_parking.rs`, `commands/gsx.rs`, `commands/gsx_remote.rs`): catálogo de perfiles de flightsim.to (~2.700), búsqueda por ICAO, instalación, variantes, updates de perfiles y de GSX, gate desde el menú de GSX o perfiles, recibos para economía, GSX remoto (descubre por LAN el puerto 8744).
- **Navigraph/AIRAC** (`airac.rs`): calendario fijo de 28 días anclado a 2301 = 2023-01-26; el ciclo **instalado se lee de la navdata real** (`Community/navigraph-nav-*/manifest.json`, p.ej. "AIRAC Cycle 2610 rev.1"); aviso el día efectivo (fecha local); re-chequeo al arrancar, al enfocar y a medianoche; abre Navigraph Hub.
- **Discord RPC** (`discord_rpc.rs`), **notificaciones nativas** (plugin notification), **autoarranque** (plugin autostart, `HKCU\…\Run`).
- **Updater** (`commands/updater.rs`, `updater.rs`): consulta `api.github.com/repos/Daniiel18/msfs-addons-browser/releases/latest`, compara semver con `CARGO_PKG_VERSION`, descarga el instalador NSIS a `%LOCALAPPDATA%\SimFleet\updater-cache` y lo lanza; **no** comprueba updates en builds locales (`target/`) salvo `SIMFLEET_FORCE_UPDATE_CHECK=1`.
- **Supabase** (proyecto `simfleet-vs`, solo anon key pública + RLS): **Live VS** (Realtime presence/broadcast por canal `simfleet-vs-<fecha>-<origen>-<destino>`, posición del rival en el mapa, FPM y avión), tabla `vs_results` (resultado persistente por vuelo+piloto para duelos asíncronos), tabla `pilot_avatars` (foto del piloto). En pruebas: tablas `*_test` y canales `simfleet-test-vs-*`.

### 6.7 Best Landings (grabación) y OSD
- `landing_recorder.rs`: arma la captura en aproximación (fase approach/descent o <3000 ft tras haber estado en crucero), graba a disco con **Windows Graphics Capture** + encoder **H.264 por hardware** (crate `windows-capture` 2) + audio del sistema por **WASAPI loopback**, guarda el clip ~12 s tras el touchdown, tope de 12 min armado, manifest de clips con FPM/aeropuerto/avión, máximo N clips (rota los viejos). Elige la ventana más grande del proceso del sim y, si en 6 s no llegan frames, captura su monitor. Lock compartido entre instancias (la principal tiene prioridad). Carpeta por defecto `Videos\SimFleet Landings` (en pruebas `… - PRUEBAS`); hereda config de LandingToast si existe.
- OSD: ventana `osd` transparente que muestra FPM y grado al aterrizar (`setWindowPos` HWND_TOPMOST sin robar foco).

### 6.8 Nube: Google Drive
- `cloud_sync.rs` (3.000 líneas) + `commands/cloud.rs`. OAuth de **app de escritorio** con PKCE y listener loopback en `127.0.0.1:<puerto libre>`; scopes `drive.appdata` + `userinfo.email`; allowlist de cuentas por hash SHA-256 (`owner_ids.rs`); credenciales embebidas en compilación desde `src-tauri/secrets.local.toml` (gitignored; `build.rs`).
- Snapshot único `msfs-addons-data.json` (JSON gzip, cientos de MB con tracks) en el **appDataFolder** de Drive: `flightLog`, `flightLogTrack`, `flightLogPhase`, `flightLogScoreItem`, `settings` (solo `pref_*`).
- **Semántica** (importante):
  - *Subir* (`upload_all`): descarga el snapshot remoto, **añade** los vuelos locales que no estén (claves estables) con ids nuevos y remapea tracks/fases/score; **nunca borra ni reemplaza** vuelos remotos; settings: manda lo local. No re-sube si no hay nada nuevo.
  - *Bajar* (`download_missing` → `merge_remote_snapshot`): **añade** los vuelos remotos que faltan; nunca borra ni sobrescribe locales; respeta el índice VA (el nº de vuelo lo conserva el vuelo más largo del grupo); settings: manda la nube.
  - *Sync* / auto-sync diario: DB vacía → `restore_snapshot` (por id, misma política VA); con datos → bajar + subir.
  - *Purgar*: borra solo la copia de la nube. *Carpeta local*: guarda/carga el mismo JSON en una carpeta (OneDrive, etc.); cargar con vuelos locales hace merge seguro.
  - Limitación: editar un vuelo que ya está en la nube no propaga la edición.
- Sesión expirada (`invalid_grant`): se descarta el refresh token, se marca `google_session_expired` y la UI pide reconectar (sin reintentos en cada vuelo). `invalid_client` (secret rotado) da un mensaje explícito.

### 6.9 Diagnóstico
- `diagnostics.rs` + `DiagnosticsSettings.tsx`: árbol de procesos de SimFleet y RAM, actividad en vivo de subsistemas (SimConnect, recorder, cloud, descargas), comando de prueba de conexión a Drive paso a paso.

## 7. Datos

### 7.1 Ubicaciones
- **Instalación real** (equipo del dueño): `B:\Program Files\SimFleet\simfleet.exe`, datos en `B:\Program Files\SimFleet\data\` (DB `msfs-addons.db` + WAL, `logs/`, `temp/`, `torrents/`, `extensions/` con uBOL). MSFS 2024 en `B:\Microsoft Flight Simulator 2024` (Community en `…\Community`).
- **Entorno de pruebas**: `%LOCALAPPDATA%\SimFleet-Pruebas\data` (+ `webview2/` como perfil de WebView2).
- Clips: `%USERPROFILE%\Videos\SimFleet Landings[ - PRUEBAS]`. Caché del updater: `%LOCALAPPDATA%\SimFleet\updater-cache`. Lock del recorder: `%LOCALAPPDATA%\SimFleet\simfleet_recorder[.pruebas].lock`.

### 7.2 Base de datos
Esquema completo en el **Apéndice H** (generado de una DB totalmente migrada). Tablas clave: `flight_log` (vuelo: OOOI, origen/destino, avión, FPM, distancia, scoring, VA: airline/flight_number/callsign/status, SimBrief, METAR…), `flight_log_track` (muestras ~2–5 s con ~45 columnas de telemetría), `flight_log_phase`, `flight_log_score_item`, `community_packages` (inventario escaneado), `addons` (caché de catálogos), `download_jobs`/`downloads`/`installed_addons`, `airports`, `settings` (k/v), `simbrief_flights`, `gsx_lookups`, `update_check_cache`, `dismissed_updates`, `flightsim_tracked_files`, economía (`airline_economy`, `airline_policy`, `flight_pnl`), `maintenance_log`, `logs`, enlaces entre addons (`addon_links`, `addon_link_positions`). 23 tablas en total.

### 7.3 localStorage (por máquina, no se sincroniza)
`simfleet.locale`, `simfleet.whatsnew.seen.v5`, `sf_splash_update_skipped`, `msfs-addons-browser:updater:dismissed-version`, `simfleet:skip-version-prompt-once`, `simfleet.addons.view`, `simfleet.map.filter`, `simfleet.map.gsxFilter`, `simfleet.map.sidebarCollapsed`, `simfleet.fb.mapCamera`, `simfleet.fb.selectedFlightId`, `simfleet.fb.sidebarCollapsed`, `simfleet.fps.applied.v1`, `simfleet.fps.known.v3`, `simfleet.fps.notices.v1`, `simfleet.gsxoffer.known.v1`, `simfleet:gsx-update-dismissed-version`, `simfleet.maint.dismissed.v1`, `simfleet.newscenery.dismissed.v1`, `simfleet.newscenery.seen_ms.v1`, `simfleet.preflight.alerted_ofp.v1`, `simfleet.preflight.bypass.v1`, `simfleet.vs.notices.v1`, `simfleet.vs.<canal>` (duelos), `simfleet:pilot-avatar:<identidad>`, `simfleet:planespotters-cache:v1`, `simfleet.diag.unlocked.v1`, `msfs-addons:notifications-peeked`, `sf_native_notifications`.

## 8. Build, desarrollo y release
- Requisitos: Node 20+ (hay 24), Rust stable (hay 1.99; `rust-version = 1.77`), VS 2022 Build Tools (C++), WebView2 (runtime 155).
- `npm run dev` (solo UI, demo), `npm run tauri dev` (app completa; Vite en 1420), `npm run build` (`tsc -b && vite build`), `npm run tauri:build`, `scripts/build-release.ps1` (limpia `target/` conservando `data/` y genera el instalador NSIS `SimFleet_<versión>_x64-setup.exe`).
- **Secretos**: `src-tauri/secrets.local.toml` (gitignored) con `SIMFLEET_GOOGLE_CLIENT_ID/SECRET`; `build.rs` los inyecta como `rustc-env` y `cloud_sync.rs` los lee con `option_env!`. Sin ese archivo, Drive queda deshabilitado. Ojo: el watcher de `tauri dev` ignora archivos gitignored (tocar un `.rs` para recompilar).
- CI: `.github/workflows/pages.yml` (publica el demo con `VITE_DEMO=1`, base `/msfs-addons-browser/`), `release.yml` (al empujar un tag `v*` crea un release borrador con notas del CHANGELOG y verifica que las versiones de `package.json`, `tauri.conf.json` y `Cargo.toml` coincidan; el instalador se compila en local y se adjunta a mano).
- Versiones a mantener sincronizadas: `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` (la 7.4.0 olvidó Cargo.toml y causó un bucle de updates).
- Calidad hoy: `tsc` 0 errores; `vite build` OK (bundle principal 3,5 MB, aviso de chunk grande); `cargo test --lib` 86 OK / 4 ignorados (incluye tests de AIRAC, merge de Drive, etiquetado de Simplaza); `cargo clippy` 0 errores / 97 avisos de estilo.

## 9. Seguridad y privacidad
- Credenciales OAuth de app de escritorio embebidas en el binario (no son confidenciales en apps de escritorio, pero el secret se **rotó** el 2026-10-18; el instalador 7.4.1 lleva el viejo, ya inválido).
- Supabase con anon key pública y RLS abierta (app privada de 2 personas).
- Correos de cuentas permitidas guardados solo como hash SHA-256.
- Navegadores embebidos con ad-block de red + uBOL; descargas a carpeta temporal propia y auto-borrado tras instalar.
- La app instala con acceso total a la Community real; el entorno de pruebas también (decisión del dueño).

## 10. Glosario
OOOI (Out-Off-On-In: block-out, despegue, aterrizaje, block-in) · OFP (plan de vuelo de SimBrief) · FPM (velocidad vertical al tocar) · Community (carpeta de addons de MSFS) · Official (paquetes oficiales) · GSX (FSDreamTeam Ground Services) · AIRAC (ciclo de 28 días de navdata) · VA (aerolínea virtual) · VAS-ACARS (cliente ACARS de terceros) · LVar (variable local de avión, vía MobiFlight) · Turnstile (captcha de Cloudflare) · uBOL (uBlock Origin Lite) · WGC (Windows Graphics Capture).

## 11. Entorno de pruebas (instancia secundaria)
Se lanza con `.\dev-safe.ps1` (UTF-8 con BOM) o `npm run dev:safe`; `-SinSimConnect` lo deja sin watcher. Código: `src-tauri/src/test_env.rs`, `src/lib/testEnv.ts`. Corre **a la vez** que la app instalada y nunca mezcla datos:
- Datos propios en `%LOCALAPPDATA%\SimFleet-Pruebas\data` con marca `ENTORNO_DE_PRUEBAS.txt` (doble candado: pruebas no adopta un dir con DB sin marca; la principal no abre un dir marcado). Perfil WebView2 propio.
- Drive: **baja** del snapshot real (solo lectura) y **sube/purga** solo `msfs-addons-data.PRUEBAS.json`; auto-sync activo.
- Supabase: tablas `vs_results_test`/`pilot_avatars_test` y canales `simfleet-test-vs-*`.
- Autoarranque propio "SimFleet Pruebas" (`--pruebas`), clips en carpeta propia, lock de grabación que cede a la principal.
- **No** aislados por decisión del dueño: Community real (acceso total), SimConnect (activo), Discord.
- Indicadores: título "SimFleet — ENTORNO DE PRUEBAS", banner rojo con la ruta de datos, insignia "Pruebas", tooltip de bandeja.

## 12. Cambios recientes sin commitear (v7.4.2, 2026-10-18)
1. **Entorno de pruebas** completo (§11) + título de ventana desde Rust (faltaba el permiso `set-title`).
2. **Best Landings** nunca había guardado un clip ("0 frames"): selección de ventana por proceso + plan B de captura de monitor; log "clip guardado" solo si de verdad se guardó; lock entre instancias realmente compartido.
3. **Drive**: `invalid_grant` → sesión expirada sin reintentos; `invalid_client` → mensaje claro; bajada/restore/carga de carpeta respetan el índice VA y deduplican dentro del snapshot (error "UNIQUE constraint failed: uq_flight_log_va_dedup_completed"); carga de carpeta con merge seguro; funciones `merge_remote_snapshot`, `insert_cloud_flight`, `upsert_snapshot_flight`, `cloud_va_key`.
4. **Fuentes**: Skybound oculto (HTTP 530) con cortacircuitos en el escáner; Simplaza visible en MSFS 2024 con etiquetado por título.
5. **AIRAC**: ciclo instalado leído de la navdata de Navigraph; validación de fechas con tests contra el calendario OACI; re-chequeo al enfocar y a medianoche.
6. **Mapa**: terminator reescrito (antes ~95 % de error en bandas −12°/−18° cerca de equinoccios); resincroniza al volver a la ventana; enfoque del vuelo seleccionado esperando su track real y re-encuadre si el contenedor cambia de tamaño.
7. **Mapas sin "API KEY REQUIRED"**: CARTO sustituido por Esri; glyphs añadidos al estilo satélite (los idents de los fixes no se dibujaban).
8. **Tarjeta compartida**: fondo opaco (esquinas blancas en chats), textos medidos y ajustados (nombres cortados), línea de ruta visible (gradiente en userSpaceOnUse).
9. **Liveries**: menú por modelo para paquetes con varios aviones (Fenix A319 & A321).
10. **Dashboard**: tarjeta "Addons en disco".
11. **Navegadores embebidos / Turnstile**: sin UA falso en WebView; scripts inyectados solo en el frame principal; filtro de red solo para dominios de anuncios.
12. **Otros**: duplicado de comprobación de updates, `ReplayBanner` en demo, avisos de compilación, clippy `modulo_one`.

## 13. Problemas conocidos y pendientes
- **Turnstile/mixdrop**: corregidas tres causas (UA falso, scripts en el iframe, filtro "*"); falta confirmarlo con una descarga real. Si persistiera, el siguiente sospechoso es uBOL Lite dentro de la ventana de hosters, o las propias inyecciones de IPC de Tauri en subframes.
- **Recorder**: arreglo de selección de ventana sin probar en vuelo real.
- **SimConnect** `EXCEPTION DATA_ERROR sendID=93 index=2` al cargar avión con el sim en pausa (sin etiquetar; inofensivo).
- **App instalada 7.4.1**: Drive roto (secret viejo embebido); hay que publicar una versión con el secret nuevo.
- **OAuth en modo "Testing"** (hipótesis): Google caduca los refresh tokens a los 7 días → `invalid_grant` recurrente; revisar en Google Cloud Console y publicar la app.
- Ediciones de vuelos no se propagan a la nube (subir solo añade).
- `SimBriefConfirmModal.tsx` escucha `simbrief://confirm`, pero el backend ya no lo emite: al apagar motores vincula automáticamente el OFP más reciente del piloto (filtro de propiedad por la cuenta de Drive). El modal es código muerto o una regresión a revisar. Igual `score:error` y `score:upload:started` se emiten y nadie los escucha.
- Bundle frontend grande (3,5 MB) y componentes enormes (FlightBookView 2.600, SettingsModal 2.300, HangarView 2.400, RoutesMapView 2.100 líneas); `simconnect_watcher.rs` 5.600 líneas.
- 97 avisos de clippy (estilo); funciones con muchos argumentos.
- Dependencia de scraping de sitios de terceros (SceneryAddons, Simplaza, flightsim.to) que cambian sin aviso.
- El dueño tiene ideas para **rediseñar la UI** (fase futura).

---

# PARTE II — Inventarios generados del código (2026-10-18)


## Apéndice A — Comandos Tauri (frontend → backend vía `invoke`)

Total: **152 comandos** registrados en `generate_handler!` (`src-tauri/src/lib.rs`). Se invocan desde el frontend con `invoke("nombre")` a través de `src/lib/tauri.ts`.

- **commands::search** (3): `search`, `list_sources`, `browse_source`
- **commands::addons** (1): `list_installed`
- **commands::downloads** (6): `list_downloads`, `start_download`, `cancel_download`, `pause_download`, `resume_download`, `clear_download`
- **commands::install** (3): `community_folder`, `install_archive`, `forget_install`
- **commands::gsx** (6): `gsx_lookup`, `gsx_list_installed_icaos`, `gsx_check_profile_updates`, `gsx_ack_update`, `gsx_install_profile`, `read_text_file`
- **commands::livery** (6): `start_livery_download_watch`, `open_livery_browser`, `save_installer_to`, `flightsim_check_updates`, `flightsim_ack_update`, `flightsim_download_meta`
- **gsx_update** (2): `gsx_check_update`, `gsx_set_installed_version`
- **airac** (3): `airac_check_update`, `airac_set_installed_cycle`, `airac_updater_path`
- **planespotters** (1): `aircraft_photo`
- **commands::updater** (2): `check_for_update`, `install_update`
- **commands::airports** (3): `list_addons_on_map`, `refresh_airports_dataset`, `lookup_airports`
- **commands::community** (25): `scan_community`, `get_installed_sims`, `community_targets`, `pack_liveries`, `is_safe_mode`, `test_env_data_dir`, `is_watcher_active`, `cross_link_create`, `list_community_packages`, `list_available_updates`, `refresh_updates_for_installed`, `uninstall_community_package`, `diagnose_update_for_package`, `dismiss_update`, `dismiss_all_updates`, `clear_dismissed_updates`, `package_thumbnail`, `list_pmdg_liveries`, `set_package_enabled`, `set_packages_enabled`, `list_addon_links`, `add_addon_link`, `remove_addon_link`, `list_addon_node_positions`, `save_addon_node_positions`
- **diagnostics** (2): `diagnostics_process_tree`, `diagnostics_app_metrics`
- **commands::perf** (5): `perf_read_config`, `perf_toggle_option`, `perf_enrich_from_source`, `perf_list_optimizable`, `perf_scan_all_from_source`
- **commands::changelog** (1): `fetch_changelog`
- **commands::simbrief** (8): `get_simbrief_pilot_id`, `set_simbrief_pilot_id`, `refresh_simbrief`, `simbrief_briefing`, `list_simbrief_flights`, `delete_simbrief_flight`, `link_simbrief_ofp`, `flight_route_fixes`
- **commands::stats** (4): `get_dashboard_stats`, `get_addon_health`, `health_move_to_sim`, `health_prune_tracked`
- **commands::flight_log** (20): `list_flight_log`, `hangar_analytics`, `pilot_profile`, `list_achievements`, `list_incomplete_flights`, `keep_partial_flight`, `delete_flight_log_entry`, `force_close_flight_log_entry`, `debug_seed_flight_log`, `get_flight_status`, `get_flight_track`, `get_flight_track_full`, `save_flight_metar`, `analyze_flight_damage`, `update_flight_log_entry`, `msfs_logbook_detect`, `msfs_logbook_import`, `vas_acars_detect`, `vas_acars_import`, `delete_flights_by_source`
- **commands::economy** (9): `airline_economy`, `airline_balance_history`, `airline_policy`, `set_airline_policy`, `airline_fleet_maintenance`, `service_aircraft`, `aircraft_maintenance`, `aircraft_maintenance_history`, `maintenance_alerts`
- **commands::recording** (9): `recording_config`, `recording_engine_status`, `osd_test`, `osd_debug`, `osd_show_topmost`, `recording_test_clip`, `list_landing_clips`, `set_landing_favorite`, `delete_landing_clip`
- **commands::settings** (6): `get_app_settings`, `set_app_setting`, `set_backend_locale`, `set_autostart`, `clear_caches`, `reset_settings`
- **commands::backup** (3): `backup_community`, `export_addons`, `save_binary_file`
- **commands::skybound** (2): `skybound_set_credentials`, `skybound_status`
- **commands::gsx_remote** (1): `gsx_remote_status`
- **commands::cloud** (13): `cloud_get_config`, `cloud_set_credentials`, `cloud_start_oauth`, `cloud_disconnect`, `cloud_sync_now`, `cloud_upload_all`, `cloud_download_missing`, `cloud_test_connection`, `cloud_purge`, `folder_sync_get_config`, `folder_sync_save`, `folder_sync_load`, `folder_sync_clear`
- **commands::drop** (4): `drop_inspect`, `drop_commit`, `drop_cancel`, `delete_dropped_archive`
- **commands::scoring** (4): `score_flight`, `score_get_report`, `list_airlines`, `airline_kpis`

## Apéndice B — Métodos de la capa `Api` del frontend (`método`→`comando`)

Interfaz `Api` en `src/lib/tauri.ts`: **167 métodos**. `realApi` los implementa con `invoke()` de Tauri; `demoApi` con datos falsos para el demo web (GitHub Pages). `export const api = isTauri ? realApi : demoApi`.

`listSources`→`list_sources`, `skyboundSetCredentials`→`skybound_set_credentials`, `skyboundStatus`→`skybound_status`, `gsxRemoteStatus`, `search`→`search`, `browseSource`→`browse_source`, `openExternal`, `openLocalPath`, `listDownloads`→`list_downloads`, `startDownload`→`start_download`, `cancelDownload`→`cancel_download`, `pauseDownload`→`pause_download`, `resumeDownload`→`resume_download`, `clearDownload`→`clear_download`, `onDownloadUpdate`, `communityFolder`→`community_folder`, `installFromFile`, `listInstalled`→`list_installed`, `forgetInstall`→`forget_install`, `gsxLookup`→`gsx_lookup`, `gsxListInstalledIcaos`→`gsx_list_installed_icaos`, `gsxCheckProfileUpdates`→`gsx_check_profile_updates`, `gsxAckUpdate`→`gsx_ack_update`, `flightsimCheckUpdates`→`flightsim_check_updates`, `flightsimAckUpdate`→`flightsim_ack_update`, `listAchievements`→`list_achievements`, `flightsimDownloadMeta`→`flightsim_download_meta`, `communityTargets`→`community_targets`, `startLiveryDownloadWatch`→`start_livery_download_watch`, `openLiveryBrowser`→`open_livery_browser`, `saveInstallerTo`→`save_installer_to`, `gsxInstallProfile`→`gsx_install_profile`, `readTextFile`→`read_text_file`, `aircraftPhoto`→`aircraft_photo`, `gsxCheckUpdate`→`gsx_check_update`, `gsxSetInstalledVersion`→`gsx_set_installed_version`, `airacCheckUpdate`→`airac_check_update`, `airacSetInstalledCycle`→`airac_set_installed_cycle`, `airacUpdaterPath`→`airac_updater_path`, `cloudGetConfig`→`cloud_get_config`, `cloudSetCredentials`→`cloud_set_credentials`, `cloudStartOauth`→`cloud_start_oauth`, `cloudDisconnect`→`cloud_disconnect`, `cloudSyncNow`→`cloud_sync_now`, `cloudUploadAll`→`cloud_upload_all`, `cloudDownloadMissing`→`cloud_download_missing`, `cloudTestConnection`→`cloud_test_connection`, `cloudPurge`→`cloud_purge`, `msfsLogbookImport`→`msfs_logbook_import`, `vasAcarsImport`→`vas_acars_import`, `vasAcarsDetect`→`vas_acars_detect`, `deleteFlightsBySource`→`delete_flights_by_source`, `onCloudOauthCompleted`, `onCloudProgress`, `folderSyncGetConfig`→`folder_sync_get_config`, `folderSyncSave`→`folder_sync_save`, `folderSyncLoad`→`folder_sync_load`, `folderSyncClear`→`folder_sync_clear`, `dropInspect`→`drop_inspect`, `dropCommit`→`drop_commit`, `dropCancel`→`drop_cancel`, `deleteDroppedArchive`→`delete_dropped_archive`, `checkForUpdate`, `installUpdate`→`install_update`, `onUpdateProgress`, `listAddonsOnMap`→`list_addons_on_map`, `refreshAirportsDataset`→`refresh_airports_dataset`, `lookupAirports`→`lookup_airports`, `flightRouteFixes`→`flight_route_fixes`, `scanCommunity`→`scan_community`, `getInstalledSims`→`get_installed_sims`, `isSafeMode`→`is_safe_mode`, `isWatcherActive`→`is_watcher_active`, `testEnvDataDir`→`test_env_data_dir`, `diagnosticsProcessTree`→`diagnostics_process_tree`, `diagnosticsAppMetrics`→`diagnostics_app_metrics`, `maintenanceAlerts`→`maintenance_alerts`, `crossLinkCreate`→`cross_link_create`, `onCrossLinkOffer`, `listCommunityPackages`→`list_community_packages`, `packLiveries`→`pack_liveries`, `listAvailableUpdates`→`list_available_updates`, `refreshUpdatesForInstalled`→`refresh_updates_for_installed`, `uninstallCommunityPackage`→`uninstall_community_package`, `diagnoseUpdateForPackage`→`diagnose_update_for_package`, `packageThumbnail`→`package_thumbnail`, `listPmdgLiveries`→`list_pmdg_liveries`, `setPackageEnabled`→`set_package_enabled`, `setPackagesEnabled`→`set_packages_enabled`, `listAddonLinks`→`list_addon_links`, `addAddonLink`→`add_addon_link`, `removeAddonLink`→`remove_addon_link`, `listAddonNodePositions`→`list_addon_node_positions`, `saveAddonNodePositions`→`save_addon_node_positions`, `perfReadConfig`→`perf_read_config`, `perfToggleOption`→`perf_toggle_option`, `perfEnrichFromSource`→`perf_enrich_from_source`, `perfListOptimizable`→`perf_list_optimizable`, `perfScanAllFromSource`→`perf_scan_all_from_source`, `onThumbsUpdated`, `fetchChangelog`→`fetch_changelog`, `getSimbriefPilotId`→`get_simbrief_pilot_id`, `setSimbriefPilotId`→`set_simbrief_pilot_id`, `refreshSimbrief`→`refresh_simbrief`, `simbriefBriefing`→`simbrief_briefing`, `listSimbriefFlights`→`list_simbrief_flights`, `deleteSimbriefFlight`→`delete_simbrief_flight`, `linkSimbriefOfp`→`link_simbrief_ofp`, `dismissUpdate`→`dismiss_update`, `dismissAllUpdates`→`dismiss_all_updates`, `clearDismissedUpdates`→`clear_dismissed_updates`, `getDashboardStats`→`get_dashboard_stats`, `getAddonHealth`→`get_addon_health`, `healthMoveToSim`→`health_move_to_sim`, `healthPruneTracked`→`health_prune_tracked`, `getAppSettings`→`get_app_settings`, `setAppSetting`→`set_app_setting`, `setBackendLocale`→`set_backend_locale`, `setAutostart`→`set_autostart`, `clearCaches`→`clear_caches`, `resetSettings`→`reset_settings`, `backupCommunity`→`backup_community`, `exportAddons`→`export_addons`, `pickSavePath`, `saveBinaryFile`→`save_binary_file`, `pickFolderPath`, `pickFilePath`, `pickFilePaths`, `listFlightLog`→`list_flight_log`, `getFlightTrack`→`get_flight_track`, `getFlightTrackFull`→`get_flight_track_full`, `saveFlightMetar`→`save_flight_metar`, `listIncompleteFlights`→`list_incomplete_flights`, `keepPartialFlight`→`keep_partial_flight`, `analyzeFlightDamage`→`analyze_flight_damage`, `updateFlightLogEntry`→`update_flight_log_entry`, `deleteFlightLogEntry`→`delete_flight_log_entry`, `debugSeedFlightLog`→`debug_seed_flight_log`, `onFlightLogChange`, `getFlightStatus`→`get_flight_status`, `onFlightStatus`, `scoreFlight`→`score_flight`, `scoreGetReport`→`score_get_report`, `listAirlines`→`list_airlines`, `airlineKpis`→`airline_kpis`, `hangarAnalytics`→`hangar_analytics`, `airlineEconomy`→`airline_economy`, `airlineBalanceHistory`→`airline_balance_history`, `airlinePolicy`→`airline_policy`, `setAirlinePolicy`→`set_airline_policy`, `airlineFleetMaintenance`→`airline_fleet_maintenance`, `aircraftMaintenance`→`aircraft_maintenance`, `aircraftMaintenanceHistory`→`aircraft_maintenance_history`, `serviceAircraft`→`service_aircraft`, `pilotProfile`→`pilot_profile`, `recordingConfig`→`recording_config`, `recordingEngineStatus`→`recording_engine_status`, `osdTest`→`osd_test`, `osdDebug`→`osd_debug`, `osdShowTopmost`→`osd_show_topmost`, `recordingTestClip`→`recording_test_clip`, `listLandingClips`→`list_landing_clips`, `setLandingFavorite`→`set_landing_favorite`, `deleteLandingClip`→`delete_landing_clip`, `onScoreDone`, `onScoreUploadSuccess`, `onScoreUploadError`

## Apéndice C — Eventos backend → frontend

Nombres de evento encontrados como literales en el código (incluye los emitidos con constantes y en varias líneas).

| Evento | Backend (Rust) | Frontend |
|---|---|---|
| `cloud://oauth-completed` | src-tauri/src/cloud_sync.rs | src/lib/tauri.ts |
| `cloud://progress` | src-tauri/src/cloud_sync.rs | src/lib/tauri.ts |
| `cross-link://offer` | src-tauri/src/commands/drop.rs, src-tauri/src/download/manager.rs | src/lib/tauri.ts |
| `download://update` | src-tauri/src/download/manager.rs | src/lib/tauri.ts |
| `embedded-download://finished` | src-tauri/src/commands/livery.rs | src/components/DragDropOverlay.tsx |
| `embedded-download://started` | src-tauri/src/commands/livery.rs | src/components/DragDropOverlay.tsx |
| `flight://current` | src-tauri/src/simconnect_watcher.rs | src/lib/tauri.ts |
| `flight://replay` | src-tauri/src/simconnect_watcher.rs | src/components/ReplayBanner.tsx |
| `flightlog://changed` | src-tauri/src/commands/flight_log.rs, src-tauri/src/commands/simbrief.rs, src-tauri/src/scoring/mod.rs, src-tauri/src/simconnect_watcher.rs | src/components/EconomyView.tsx, src/lib/tauri.ts, src/stores/useMaintAlertsStore.ts |
| `flightlog://incomplete` | src-tauri/src/simconnect_watcher.rs | src/components/IncompleteFlightModal.tsx |
| `landing://osd` | src-tauri/src/commands/recording.rs, src-tauri/src/simconnect_watcher.rs | src/components/LandingOsd.tsx, src/components/LiveVsManager.tsx |
| `livery-download://finished` | src-tauri/src/commands/livery.rs | src/components/DragDropOverlay.tsx |
| `score:done` | src-tauri/src/scoring/mod.rs | src/lib/tauri.ts |
| `score:error` | src-tauri/src/scoring/mod.rs | — |
| `score:upload:error` | src-tauri/src/scoring/mod.rs | src/lib/tauri.ts |
| `score:upload:started` | src-tauri/src/commands/flight_log.rs, src-tauri/src/scoring/mod.rs | — |
| `score:upload:success` | src-tauri/src/commands/flight_log.rs, src-tauri/src/scoring/mod.rs | src/lib/tauri.ts |
| `simbrief://confirm` | — | src/components/SimBriefConfirmModal.tsx |
| `thumbs://updated` | src-tauri/src/airport_thumbs.rs | src/lib/tauri.ts |
| `updater://progress` | src-tauri/src/commands/updater.rs | src/lib/tauri.ts |

## Apéndice D — Módulos Rust (`src-tauri/src`)

| Archivo | Líneas | fns | cmds | Propósito (doc del módulo) |
|---|---:|---:|---:|---|
| `src-tauri/src/achievements.rs` | 430 | 3 | 0 | (v7) **Logros / insignias** — se calculan íntegramente a partir del historial de vuelos que la app ya tiene (`flight_log`), sin telemetría nueva ni tablas nuevas. Cada logro tiene niveles ascendentes (bronce → diamante) y devuelve su valor actual + el nivel al… |
| `src-tauri/src/airac.rs` | 340 | 15 | 3 | (v6.2.8) Avisos de actualización del **AIRAC** (datos de navegación). A diferencia de GSX, el AIRAC NO necesita scrapear: los ciclos siguen un calendario FIJO de 28 días anclado a una fecha de referencia conocida (AIRAC 2301 efectivo el 2023-01-26, validado co… |
| `src-tauri/src/aircraft_maintenance.rs` | 771 | 16 | 0 | (v6.1) Mantenimiento por aeronave — fuente ÚNICA compartida por el Hangar (silueta con zonas de desgaste) y por Finanzas (panel de servicio tipo EFB). El desgaste de cada componente se deriva de los VUELOS de esa matrícula desde el último servicio (estilo EFB … |
| `src-tauri/src/airline_economy.rs` | 1756 | 42 | 0 | (v6.1) Economía de aerolíneas — módulo backend, **sin pantalla**. Idea: cada aerolínea arranca con un VALOR BASE realista (su "banco": market-cap real aproximado 2024 o un estimado en miles de millones — nunca cifras de juguete tipo 600 USD), y a partir de ahí… |
| `src-tauri/src/airport_thumbs.rs` | 387 | 7 | 0 | (v4.26.0) Descarga de imágenes para aeropuertos sin thumbnail. Muchos packs de scenery traen el PNG "PLACEHOLDER" del SDK o directamente ninguna imagen. Este módulo busca una foto real en **Wikipedia** (REST API `page/summary`, que resuelve redirects: "SUMU" →… |
| `src-tauri/src/airports.rs` | 558 | 12 | 0 | Dataset de aeropuertos para la vista de mapa. Origen: OurAirports (CC0). El mirror estable de GitHub Pages es `https://davidmegginson.github.io/ourairports-data/airports.csv`, pesa ~7MB y trae ~80k filas. Filtramos por `type` (descartamos heliports/closed/seap… |
| `src-tauri/src/cloud_sync.rs` | 2975 | 62 | 0 | Sincronización con Google Drive (v2.0.0). ## Flujo 1. **Credentials**: el usuario pega su Client ID + Client Secret de un OAuth client tipo "Desktop app" creado en Google Cloud Console. No podemos shippearlas porque Google no permite distribuir credenciales de… |
| `src-tauri/src/commands/addons.rs` | 10 | 1 | 1 |  |
| `src-tauri/src/commands/airports.rs` | 47 | 3 | 3 |  |
| `src-tauri/src/commands/backup.rs` | 344 | 10 | 3 | Backup y export de la carpeta Community. · `backup_community` — comprime la carpeta Community en un .zip con timestamp en el nombre, en una ruta que el usuario elige. Útil antes de actualizar MSFS o instalar packs experimentales. · `export_addons_list` — expor… |
| `src-tauri/src/commands/changelog.rs` | 224 | 4 | 1 |  |
| `src-tauri/src/commands/cloud.rs` | 164 | 13 | 13 | Comandos Tauri para la sincronización con Google Drive. Mapean 1:1 con funciones de `crate::cloud_sync`. Sólo se ocupan de: · Aceptar parámetros del frontend. · Mapear errores a `Result<_, String>` (Tauri serializa). · Loguear via `CmdTimer` para diagnóstico. |
| `src-tauri/src/commands/community.rs` | 729 | 32 | 25 |  |
| `src-tauri/src/commands/downloads.rs` | 84 | 6 | 6 |  |
| `src-tauri/src/commands/drop.rs` | 181 | 4 | 4 | Comandos Tauri para drag-and-drop universal (v2.1.0). Mapean directamente a funciones de `crate::drop_install`. La sesión + tempdir vive en `AppState::drop_sessions` (Mutex). |
| `src-tauri/src/commands/economy.rs` | 113 | 9 | 9 | (v6.1) Comando Tauri para la economía de aerolíneas. El módulo es backend-only por ahora (no hay pantalla). Este comando ya deja el contrato listo para cuando se construya la UI: recalcula el ledger consumiendo la data actual de `flight_log` (+ recibos GSX) y … |
| `src-tauri/src/commands/flight_log.rs` | 790 | 22 | 20 | Comandos Tauri para `flight_log`. Expuestos al frontend para listar el historial, leer la traza de un vuelo, borrar entradas y poblar con datos demo para validación de UI sin necesidad de MSFS corriendo. |
| `src-tauri/src/commands/gsx.rs` | 533 | 15 | 6 |  |
| `src-tauri/src/commands/gsx_remote.rs` | 179 | 6 | 1 | (v6.2.38) GSX remoto — GSX Pro puede exponer su estado en la LAN por HTTP en el puerto 8744, pero la IP rota. Por eso lo DESCUBRIMOS por sondeo: probamos la última IP buena y, si falla, escaneamos la subred /24 local buscando quién responde en 8744. De ahí sac… |
| `src-tauri/src/commands/install.rs` | 129 | 3 | 3 |  |
| `src-tauri/src/commands/livery.rs` | 1803 | 28 | 6 | (v6.2.53) Descarga de liveries de flightsim.to con la cuenta del usuario. Dos caminos coexisten: · `open_livery_browser` — navegador EMBEBIDO (WebviewWindow) a flightsim.to con captura de descarga. Salía en blanco; en v6.2.53 probamos el fix de desactivar el d… |
| `src-tauri/src/commands/mod.rs` | 24 | 0 | 0 |  |
| `src-tauri/src/commands/perf.rs` | 265 | 9 | 5 | (v5.0.0) Comandos del motor de rendimiento (FPS) por aeropuerto. El frontend pasa `install_path` (la carpeta del addon en Community) directamente — siempre lo tiene en `CommunityPackage.installPath`. Clave: para bajar la nota "Optional Configuration" correcta … |
| `src-tauri/src/commands/recording.rs` | 223 | 11 | 10 | (v6 #2b) Comandos Tauri para la grabación de "Best Landings" (motor: windows-record). |
| `src-tauri/src/commands/scoring.rs` | 226 | 4 | 4 | Comandos Tauri para el sistema de scoring (Epic E Phase H). - `score_flight` — recomputa y persiste el score del vuelo. - `score_get_report` — devuelve el score persistido SIN recomputar (rápido para abrir el ChecklistWidget). Si nunca se computó, ejecuta `sco… |
| `src-tauri/src/commands/search.rs` | 88 | 3 | 3 |  |
| `src-tauri/src/commands/settings.rs` | 373 | 12 | 6 | Comandos de configuración persistente. Todo se guarda en la tabla `settings` (k/v). La tabla ya existe desde la migración 001 y desde la 009 (`simbrief_pilot_id`); aquí añadimos las claves nuevas: · `pref_show_simconnect_lines` — bool · `pref_check_updates_on_… |
| `src-tauri/src/commands/simbrief.rs` | 272 | 8 | 8 |  |
| `src-tauri/src/commands/skybound.rs` | 89 | 4 | 2 | (v6.2.38) Comandos de Skybound: guardar credenciales + estado. Las credenciales viven en `settings` (skybound_user/skybound_pass) y se reflejan en el `skybound_auth` compartido con la fuente. |
| `src-tauri/src/commands/stats.rs` | 634 | 8 | 4 | Estadísticas agregadas para la vista «Dashboard». Toda la data sale de `community_packages` (poblada por el scanner) y de `compute_available` (cache de updates). No tocamos disco — el `size_bytes` ya viene del scan, así que el comando es barato y se puede invo… |
| `src-tauri/src/commands/updater.rs` | 535 | 7 | 2 |  |
| `src-tauri/src/community.rs` | 349 | 10 | 0 |  |
| `src-tauri/src/community_scanner.rs` | 1821 | 31 | 0 | Escáner de la carpeta Community de MSFS. Recorre cada subcarpeta inmediata, lee su `manifest.json`, y produce un `ScannedPackage` por cada paquete válido. La salida se persiste en `community_packages` (migración 005) — esa tabla es la fuente de verdad para la … |
| `src-tauri/src/cross_link.rs` | 276 | 9 | 0 | (v6 #1) Cross-linking MSFS 2020 ↔ 2024 mediante junctions NTFS. Cuando el usuario tiene **ambas** versiones de MSFS instaladas y descarga un escenario rotulado "MSFS 2020/2024", tras instalarlo en la Community de la versión ACTIVA ofrecemos crear un *junction*… |
| `src-tauri/src/damage.rs` | 175 | 1 | 0 | (v3.26.0 — P7.10) Analizador de daños / "vuelo forzado". MSFS 2020 NO expone daño estructural ni fallo de tren/flaps por SimConnect (eso es exclusivo de MSFS 2024). Lo que SÍ expone y usamos como proxy del maltrato al avión, capturado por track sample: · OVERS… |
| `src-tauri/src/db/mod.rs` | 1544 | 33 | 0 |  |
| `src-tauri/src/diagnostics.rs` | 204 | 8 | 2 | (v6.2.15) Diagnóstico de memoria — enumera el ÁRBOL de procesos de SimFleet (el binario principal + los procesos hijo del WebView2 + cualquier proceso que la app haya lanzado) con la RAM (working set) de cada uno. Sirve para cazar fugas de memoria: el Task Man… |
| `src-tauri/src/discord_rpc.rs` | 267 | 7 | 0 | (v7) **Discord Rich Presence** — muestra en tu perfil de Discord qué estás haciendo en SimFleet: "EHAM → KJFK · PMDG 737-800" con el cronómetro del vuelo, o un estado genérico cuando no estás volando. Diseño: · Vive en un `std::thread` propio (el cliente de `d… |
| `src-tauri/src/download/manager.rs` | 901 | 14 | 0 |  |
| `src-tauri/src/download/mod.rs` | 59 | 0 | 0 | Download orchestration. Every user action ("download this mirror", "grab this torrent") becomes a [`DownloadJob`] managed by [`manager::DownloadManager`]. State transitions and progress are pushed to the frontend via the Tauri event channel [`manager::EVENT_JO… |
| `src-tauri/src/download/torrent.rs` | 709 | 15 | 0 | Torrent download engine built on `librqbit`. Responsibilities of this module: * Lazy-init a single shared `Session` (DHT bootstrap + port bind are expensive; doing it per job wastes time and UDP ports). * Provide an ergonomic `download_magnet()` API that emits… |
| `src-tauri/src/drop_install.rs` | 1639 | 31 | 0 | Drag & Drop universal (v2.1.0). Reemplaza el flujo anterior de `install_archive`-only por uno que soporta: · **Perfiles GSX** (`.ini` / `.py` sueltos o dentro de `.zip`/`.rar`). · **Paquetes Community** (carpetas con `manifest.json` dentro de `.zip`/`.rar` o `… |
| `src-tauri/src/flight_log.rs` | 2153 | 40 | 0 | Registro de vuelos reales — capa de persistencia y dominio. El watcher (`simconnect_watcher.rs`) llama a `start_flight` al detectar despegue y a `finish_flight` al detectar aterrizaje. Esta capa abstrae el SQL y el lookup de aeropuerto cercano para que el watc… |
| `src-tauri/src/flight_report.rs` | 248 | 9 | 0 | (v4.19.0) **Log por vuelo** — un archivo de texto único por cada vuelo terminado, con TODAS las métricas crudas (track sample a sample: luces, flaps, gear, velocidades, fases…) más el resultado de la Flight Evaluation, para auditar si el evaluador está haciend… |
| `src-tauri/src/flightsim_track.rs` | 330 | 7 | 0 | (v6.2.60) Tracking de updates de addons descargados de flightsim.to. Forward-only y FIABLE: al instalar por el navegador EMBEBIDO registramos el `file_id` de flightsim.to del que salió cada carpeta de Community + su `updatedAt` de ese momento (baseline). El ch… |
| `src-tauri/src/gsx.rs` | 483 | 12 | 0 | Lookup de perfiles GSX Pro en flightsim.to. El sitio es una SPA detrás de Cloudflare; el HTML de `https://flightsim.to/miscellaneous/gsx-pro` es un shell vacío que hidrata su contenido vía JS. Examinando el bundle (`/assets/index-*.js`) encontramos el endpoint… |
| `src-tauri/src/gsx_menu.rs` | 195 | 11 | 0 | Lector del **menú in-sim de GSX** para rellenar el gate cuando un aeropuerto no tiene perfil de GSX. ## Cómo expone GSX el gate (verificado en el JS del panel) GSX escribe su menú a un archivo de texto plano en disco: ```text <FSDT root>\MSFS\fsdreamteam-gsx-p… |
| `src-tauri/src/gsx_parking.rs` | 529 | 16 | 0 | GSX INI parser — lee los nombres y coordenadas de los parkings desde los archivos de customization de GSX que viven en el disco. ## Por qué este approach Después de 6 hotfixes peleando con SimConnect Facility Data + GSX Client Data Areas (la DLL de MSFS 2020 r… |
| `src-tauri/src/gsx_update.rs` | 127 | 5 | 2 | (v6.2.4) Chequeo de actualizaciones de **GSX / couatl** (FSDreamTeam). FSDT no tiene API: publican las notas de cada release en `couatl_liveupdate_notes.html`, con la versión MÁS NUEVA arriba en el formato `Version X.Y.Z – fecha`. Parseamos la primera coincide… |
| `src-tauri/src/i18n.rs` | 332 | 4 | 0 | (v7.1) i18n de BACKEND. El backend no puede llamar al `t()` del frontend, así que los mensajes user-facing que devuelve (errores de comando, `DownloadJob.message/error`, payloads de eventos) salían SIEMPRE en español → un usuario con la app en inglés veía espa… |
| `src-tauri/src/install/mod.rs` | 580 | 16 | 0 | Archive extraction + install into the MSFS 2020 Community folder. Key differences vs. the legacy .NET `InstallerService`: * **Temp extraction dir is auto-cleaned.** The .NET version created a per-run folder under `%TEMP%` and never deleted it, slowly filling t… |
| `src-tauri/src/landing_recorder.rs` | 1500 | 45 | 0 | (v6 #2b · v6.2.17) Grabación nativa de "Best Landings". Motor: **windows-capture 2.x** (Windows Graphics Capture + encoder H.264 por HARDWARE vía Media Foundation) + captura WASAPI **loopback** propia para el audio del sistema. Es el MISMO stack que usa Screen… |
| `src-tauri/src/lib.rs` | 1001 | 9 | 0 |  |
| `src-tauri/src/livery_meta.rs` | 244 | 7 | 0 | (v4.23.0) Extracción INTELIGENTE de aerolínea y matrícula desde los metadatos de la livery. Problema (reportado por el usuario con el A320 FBW "LATAM PR-XBD"): los creadores de liveries rellenan `atc.cfg` con valores random — `ATC AIRLINE` suele traer el nombr… |
| `src-tauri/src/logger.rs` | 288 | 7 | 1 |  |
| `src-tauri/src/main.rs` | 6 | 1 | 0 |  |
| `src-tauri/src/mobiflight_lvars.rs` | 415 | 0 | 0 | Puente LVar **opcional** vía el módulo WASM de MobiFlight. ## Por qué existe MSFS NO expone como *simvar* nativo ni la temperatura de frenos ni ciertos datos de GSX (p. ej. el gate al que el avión está asignado cuando el aeropuerto no tiene perfil de GSX). Eso… |
| `src-tauri/src/msfs_logbook.rs` | 433 | 16 | 0 | Importador del LOGBOOK.BIN de MSFS 2020/2024. El formato del archivo está sin documentar oficialmente — es un blob binario propio de Microsoft que serializa los flight entries del logbook in-game. La comunidad lo ha reverse-engineereado parcialmente. Lo que sí… |
| `src-tauri/src/owner_ids.rs` | 47 | 3 | 0 | Identificadores privados de cuenta (hashes SHA-256). Este repositorio es PÚBLICO, así que los correos de las cuentas que usan las funciones «privadas» (allowlist del cloud sync + detección del dueño para filtros de propiedad / identidad de piloto) NO se guarda… |
| `src-tauri/src/package_ops.rs` | 527 | 10 | 0 | Operaciones sobre paquetes ya instalados en Community. Las dos primitivas que el usuario invoca desde el modal de detalle son: · **Desinstalar** — borra el folder del paquete en Community. · **Abrir carpeta** — manejado por el frontend vía `openShell`. # Desin… |
| `src-tauri/src/parser/mod.rs` | 163 | 5 | 0 |  |
| `src-tauri/src/perf_config.rs` | 1028 | 46 | 0 | (v5.0.0) Motor de rendimiento / optimización de FPS por aeropuerto. Muchos sceneries premium (Aerosoft, ORBX…) traen objetos pesados OPCIONALES que se activan/desactivan renombrando archivos `.bgl`. La receta viene en la nota "Optional Configuration" de la pág… |
| `src-tauri/src/planespotters.rs` | 66 | 2 | 1 | (v6.2.3) Resolución de la foto del avión por matrícula vía planespotters. ANTES esto se hacía con `fetch()` DESDE EL WEBVIEW. planespotters cambió su política y ahora **rechaza User-Agents genéricos** (exige uno descriptivo con URL/email de contacto). El webvi… |
| `src-tauri/src/pmdg_install.rs` | 606 | 27 | 0 | (v6.2.45) Instalador de liveries PMDG/iFly en **formato abierto** (carpeta/`.zip` con `livery.cfg` + `texture*`), portado de la app `doguer27/livery_manager_fsto` (verificada su lógica en `main.py`). Qué hace (mecanismo "A" de esa app — el que funciona bien): … |
| `src-tauri/src/pmdg_liveries.rs` | 441 | 12 | 0 | Escáner de **liveries de aircraft addons** instaladas en Community. Originalmente sólo PMDG (v3.0.0). En v3.1.0 generalizado a cubrir cualquier vendor con la convención MSFS estándar: · `{vendor}-aircraft-{model}` (base aircraft con todas las variantes adentro… |
| `src-tauri/src/scoring/mod.rs` | 823 | 10 | 0 | Sistema de puntuación de vuelo estilo Virtual Airline. Cada vuelo (SimConnect o VAS-ACARS importado) puede evaluarse contra un **rubric** de reglas agrupadas por phase. Cada regla: · Tiene un `points_max` fijo (el peso de la regla). · Lee la materia prima: la … |
| `src-tauri/src/scoring/rubric.rs` | 1989 | 71 | 0 | Rubric — define las reglas que componen el score. Cada regla tiene id estable, label visible, phase de pertenencia, peso (`points_max`) y un evaluador (closure). La regla se evalúa contra `FlightContext`. El resultado es un `ScoreItem` con puntos earned, passe… |
| `src-tauri/src/sim.rs` | 39 | 3 | 0 | (v5.1.0) Versión de MSFS activa elegida por el usuario. Afecta: · El filtro del catálogo de SceneryAddons (mostrar addons de 2020 vs 2024 según la versión elegida). · La preferencia al detectar la carpeta Community (2020 vs 2024). Es un flag global atómico par… |
| `src-tauri/src/simbrief.rs` | 1211 | 28 | 0 | Integración mínima con SimBrief. La API pública es: `https://www.simbrief.com/api/xml.fetcher.php?userid={pilotId}` Devuelve XML del **último OFP** (Operational Flight Plan) generado por ese piloto. NO hay endpoint público de historial, así que la app va const… |
| `src-tauri/src/simconnect_ffi.rs` | 637 | 2 | 0 | FFI mínimo a `SimConnect.dll` (MSFS 2020/2024). Escribimos las signatures a mano para evitar la dep de `bindgen`/libclang que arrastran los crates `simconnect` y `simconnect-sdk`. Nos basta con: · `SimConnect_Open` / `SimConnect_Close` — handshake. · `SimConne… |
| `src-tauri/src/simconnect_watcher.rs` | 5635 | 28 | 0 | Watcher de "vuelo en curso". ## Dos capas de detección 1. **SimConnect FFI** (preferida) — abre conexión real con MSFS vía `SimConnect.dll`. Suscribe simvars de posición / velocidad / on-ground y emite eventos al frontend con coordenadas en tiempo real. Cuando… |
| `src-tauri/src/sources/mod.rs` | 261 | 12 | 0 |  |
| `src-tauri/src/sources/sceneryaddons.rs` | 320 | 11 | 0 |  |
| `src-tauri/src/sources/simplaza.rs` | 514 | 19 | 0 |  |
| `src-tauri/src/sources/skybound.rs` | 685 | 19 | 0 | (v6.2.40) Fuente **Skybound** (https://skybound.cx) para MSFS2024. Simplaza no publica addons de MSFS2024, así que en esa versión del sim ofrecemos Skybound. La web es un Next.js/PayloadCMS con una **API JSON PÚBLICA** en `/api/addons` — NO hace falta login pa… |
| `src-tauri/src/test_env.rs` | 138 | 7 | 0 | (v7.4.2) ENTORNO DE PRUEBAS — 2ª instancia de SimFleet que corre junto a la principal (la instalada) SIN mezclar datos con ella. Se activa con `SIMFLEET_SAFE_MODE=1` (lo pone `dev-safe.ps1`) o con el argumento `--pruebas` (lo añade el autoarranque de la instan… |
| `src-tauri/src/updater.rs` | 193 | 2 | 0 | Comprobación de actualizaciones contra GitHub Releases. Decisión deliberada: **no** usamos `tauri-plugin-updater`. Ese plugin exige firmar los binarios con una clave privada y publicar un manifest signado al lado de cada release. Para un proyecto que aún no ti… |
| `src-tauri/src/updates.rs` | 739 | 18 | 0 | Detector de actualizaciones para paquetes instalados en Community. Combina dos pasos: 1. **Refresh activo** (`refresh_for_installed`) — para cada ICAO con versión instalada, lanza una búsqueda en cada fuente con ese ICAO como query. Concurrencia con `tokio::sy… |
| `src-tauri/src/vas_acars.rs` | 2478 | 59 | 0 | Importador de logbook de **VAS-ACARS** (Virtual Airlines System / vasystem.org). VAS-ACARS guarda cada vuelo como un archivo `.bin` independiente en `%LOCALAPPDATA%\VASystem\VAS-ACARS\flights\<uuid>.bin`. El formato es **Protocol Buffers concatenado**: un pref… |
| `src-tauri/src/vas_summary.rs` | 841 | 22 | 0 | Parser de `%LOCALAPPDATA%\VASystem\VAS-ACARS\data.db` — el índice maestro de VAS-ACARS con metadata completa por vuelo. `data.db` es una base **bbolt** (magic `0xED0CDAED`, formato Go embedded DB). Dentro de las páginas, cada vuelo se guarda como un mensaje **… |

## Apéndice E — Componentes React (`src/components`)

| Archivo | Líneas | Exporta | Descripción |
|---|---:|---|---|
| `src/components/AchievementsModal.tsx` | 196 | AchievementsModal | Orden canónico de las secciones. |
| `src/components/AddonArt.tsx` | 150 | AddonFallbackArt | (v4.29.0) Arte premium para addons sin thumbnail real. El usuario quiere: "premium oscuro con la silueta del modelo de avión correspondiente". Adiós a iniciales y iconos pequeños — ahora cada card vacía muestra una silueta GIGANTE de avión en bajo contraste so… |
| `src/components/AddonToggle.tsx` | 97 | ToggleSwitch, usePackageToggle | (v4.25.0) Interruptor físico de addon + hook de toggle. Viven en su propio archivo porque los consumen DOS vistas (las cards del grid de AddonsView y los nodos del Link Map) y meterlos en AddonsView crearía un import circular con LinkMapView. |
| `src/components/AddonsView.tsx` | 1358 | AddonsView, formatStorage | (v4.29.0) Píldoras estilo MSFS Marketplace — 5 buckets limpios. STRICT NO-AIRPORTS: cualquier paquete clasificado como SCENERY (no-airport-mods) cae en SOUND_MISC, no contamina las otras categorías. Los aeropuertos REALES viven exclusivamente en Map. |
| `src/components/AircraftMaintenance.tsx` | 470 | zoneWearOf, Silhouette, SilhouetteView, MaintenanceModal | Color por desgaste (0-100): verde → amarillo → ámbar → rojo. |
| `src/components/AirlineLogo.tsx` | 95 | AirlineLogo | (v4.9.1) Logo de aerolínea por código ICAO. Guardamos el ICAO de 3 letras por vuelo, pero los CDN de logos gratuitos (avs.io) indexan por IATA de 2 letras. Mapeamos ICAO→IATA con `airlineCodes.ts` y pedimos el bitmap a avs.io. Usamos el endpoint `al_square` (i… |
| `src/components/ChangelogModal.tsx` | 170 | ChangelogModal | URL de la página de detalle del addon (Simplaza/SceneryAddons). El backend scrapea el HTML buscando un heading "Changelog" / "What's new" / "Release notes" y devuelve las líneas. |
| `src/components/CommandPalette.tsx` | 384 | CommandPalette | Abre el modal de Ajustes (estado local de App). |
| `src/components/CrossLinkModal.tsx` | 158 | CrossLinkModal | (v6 #1) Modal post-instalación de CROSS-LINK 2020/2024. Se abre cuando el backend emite `cross-link://offer` — es decir, cuando el usuario tiene MSFS 2020 Y 2024 instalados y acaba de descargar/instalar un escenario rotulado "MSFS 2020/2024". Ofrece crear un j… |
| `src/components/DamageBadge.tsx` | 106 | DamageBadge | (v3.26.0 — P7.10) Chip con el veredicto de daños del vuelo: LIMPIO / FORZADO / DAÑADO. Se deriva de los avisos de sobrevelocidad, pérdida (stall), presión de aceite y la fuerza-G capturados por SimConnect durante el vuelo. Para vuelos viejos / importados que n… |
| `src/components/DashboardView.tsx` | 1105 | DashboardView | Dashboard — totales agregados de Community. Layout responsive: en pantallas pequeñas todo apila en columna única; en md/lg/xl crece a 4 KPIs arriba y dos columnas debajo. El bloque "más grandes" tiene un gráfico de barras horizontal con el peso real, no porcen… |
| `src/components/DiagnosticsSettings.tsx` | 366 | DiagnosticsSettings | (v6.2.15 · v6.2.16) Panel de diagnóstico — detrás de una clave de acceso. · Subsistemas INTERNOS de la app (SimConnect, tracking, nube, grabador, Crew VS) con su actividad en vivo — para cazar cuál trabaja de más. · Árbol de procesos del SO (simfleet.exe + Web… |
| `src/components/DiskCleanupModal.tsx` | 375 | DiskCleanupModal | (v6.2.29 / R6) Limpieza inteligente de disco. Cruza los escenarios de aeropuerto INSTALADOS con tu historial de vuelos: los que nunca has volado (su ICAO no aparece como origen ni destino en el FlightBook) y ocupan más disco son candidatos a desinstalar. Segun… |
| `src/components/DownloadRecoveryModal.tsx` | 112 | DownloadRecoveryModal | (v7.1) Modal de recuperación tras un fallo de descarga por TORRENT (sin seeds o timeout). Se abre solo (lo dispara `useDownloadsStore.reconcile`) y reofrece los mirrors del addon para que el usuario no quede en un callejón sin salida. El botón de torrent apare… |
| `src/components/DownloadsButton.tsx` | 42 | DownloadsButton |  |
| `src/components/DownloadsPanel.tsx` | 546 | DownloadsPanel |  |
| `src/components/DragDropOverlay.tsx` | 768 | DropRouting, DragDropOverlay | (v2.2.0) Overlay de Drag & Drop universal con modal multi-archivo. Flujo: 1. Usuario suelta N archivos. 2. La app inspecciona TODOS en paralelo (extrae cada .zip/.rar a su tempdir, clasifica los items). 3. Si SOLO hay items en 1 archivo → no abre modal, instal… |
| `src/components/DropSelectModal.tsx` | 868 | isEmbeddedDownload, DropSelectModal, DeleteConfirm | (v2.1.0, refactor v2.2.0) Modal selector multi-archivo. Antes (v2.1.x): modal por archivo. Si el usuario soltaba 3 .rar, había 3 modales secuenciales. Si uno fallaba, los siguientes quedaban orphan en la cola. Ahora (v2.2.0): UN modal con paginación interna. R… |
| `src/components/EconomyView.tsx` | 1065 | EconomyView | Compacto con signo para resultados (ej. +$3.2M, -$420.0K). |
| `src/components/EditFlightModal.tsx` | 412 | EditFlightModal | Modal para editar manualmente un vuelo ya cerrado (v0.1.25). Usado cuando el watcher detectó mal alguna métrica: · Block time inflado por una pausa larga que SimConnect no capturó (versión vieja sin el "Pause" event subscription, o pausa del SO/Tab-Alt+F4 que … |
| `src/components/ErrorBoundary.tsx` | 70 | ErrorBoundary | Last-resort runtime safety net. Without it, a throw inside any component leaves React rendering nothing and the Tauri window appears "empty" — that made Phase 2 debugging much harder than it needed to be. Surfacing the stack directly in the UI pays for itself … |
| `src/components/FindSearch.tsx` | 89 | FindSearch | (v4.33.0) Buscador estilo "find in page" del navegador: input + contador `actual/total` + flechas prev/next para navegar entre resultados + botón cerrar. La lógica de "navegar" (scroll a la card en el grid / centrar el nodo en el Link Map) la implementa cada v… |
| `src/components/FlightBookView.tsx` | 2636 | cleanAircraftLabel, FlightBookView, useAircraftPhoto | (v6.2.35) Limpia el nombre del avión para mostrar: algunas liveries cuelan " AEROLÍNEA (MATRÍCULA)" en el título ("PMDG 737-800 DAL (N374DA)"). Quitamos ese sufijo para que quede sólo el avión. No toca los nombres limpios ("Airbus", "Boeing 737-800"). |
| `src/components/FlyingNowBadge.tsx` | 306 | FlyingNowBadge | Badge "Volando ahora" — visible en el header cuando MSFS está corriendo. Tres niveles de información según lo que tengamos disponible: 1. **SimConnect activo** (mejor caso): icono `Radio` + status granular ("Cruise", "Taxi out", "Deboarding"…). En vuelo muestr… |
| `src/components/GsxOfferModal.tsx` | 238 | GsxOfferModal | Palabras genéricas que NO sirven para casar el desarrollador del escenario con el perfil (aparecen en muchos nombres). |
| `src/components/GsxRemoteBadge.tsx` | 61 | GsxRemoteBadge | (v6.2.38) Badge de estado remoto de GSX. GSX Pro expone su estado en la LAN (puerto 8744, IP que rota); lo descubrimos por sondeo. Muestra la fase (boarding/deboarding/…) + porcentaje arriba. Sólo sondea mientras el sim corre, para no escanear la subred en van… |
| `src/components/GsxSearchSummary.tsx` | 91 | GsxSearchSummary | Una sola píldora debajo del buscador con el conteo de perfiles GSX que existen en flightsim.to para el ICAO buscado. Click → abre la búsqueda en flightsim.to con `?q=ICAO`. Sólo se renderiza cuando el query es exactamente un ICAO de 4 letras ASCII — para queri… |
| `src/components/GsxUpdateGate.tsx` | 82 | GsxUpdateGate | (v6.2.6) Gate de actualización de GSX. GSX/couatl no puede actualizarse con el simulador abierto (los archivos están en uso). Cuando el usuario pide actualizar y el sim está corriendo, mostramos este warning y NO abrimos el FSDT Installer hasta detectar el sim… |
| `src/components/HangarView.tsx` | 2373 | HangarView | (v6 #2a) Vista "Hangar & Fleet Analytics" — rediseño maestro/detalle. Cabecera: título + KPIs globales (millas, aterrizajes, FPM medio) + perfil del piloto con nivel/XP. Izquierda: la flota (top 10) con buscador y selección. Derecha: detalle del avión seleccio… |
| `src/components/ImportInventoryModal.tsx` | 508 | ImportInventoryModal | (v1.1.4) Modal para importar un inventario exportado y elegir qué descargar. El usuario pidió: tras formatear el PC, importa el inventario y selecciona los addons que quiere re-instalar. Flujo: 1. Lee el archivo (CSV/TXT/JSON) y extrae títulos/ICAOs/desarrolla… |
| `src/components/IncompleteFlightModal.tsx` | 290 | IncompleteFlightModal | (v4.23.0) Modal de VUELO INCOMPLETO — aparece al abrir la app cuando hay vuelos cerrados como 'partial' sin revisar (la PC se apagó / el sim murió a mitad de vuelo y el tracking quedó cortado). Muestra el routemap con el track real recorrido + las métricas (ru… |
| `src/components/LandingOsd.tsx` | 209 | LandingOsd | (v6 #2b) OSD de aterrizaje — overlay transparente, always-on-top y click-through (estilo LandingToast). El watcher emite `landing://osd` en el MISMO instante del touchdown con FPM, G, viento, pitch y roll (todos simvars que ya leía). Esta ventana posiciona, mu… |
| `src/components/LinkMapView.tsx` | 1338 | LinkMapView | (v4.25.0) **Link Map** — grafo dirigido de dependencias de addons. · Cada nodo es una card del addon (thumbnail + badge + toggle). · Las aristas punteadas ámbar van del paquete base (aircraft) a sus dependientes (liveries, soundpacks, tweaks). Las 'auto' se si… |
| `src/components/LiveUpdateBadge.tsx` | 230 | LiveUpdateBadge | (v2.1.0) Badge "Hay update" en el header — entre el título y los controles de ventana. Aparece cuando la app está abierta y el polling periódico detecta una versión más nueva. Click → modal con changelog + botón "Instalar ahora" + checkbox "Reiniciar al termin… |
| `src/components/LiveVsManager.tsx` | 147 | LiveVsManager | (v6 #3) Componente headless que orquesta Live VS: arma el canal Supabase con la identidad del piloto + su OFP actual, y emite la posición en vivo. No pinta nada (el botón/modal/marker viven en sus componentes). Se monta una vez en App. |
| `src/components/MapAirportCard.tsx` | 384 | MapAirportCard | (v4.25.0) Card contextual del aeropuerto seleccionado en el Map (scenery). Sustituye al modal emergente: fusiona la cabecera con preview de imagen + un acordeón "Ficha técnica" con las filas del viejo modal (Folder, Version, Size, Min MSFS, Last modified, GSX)… |
| `src/components/MapView.tsx` | 892 | MapView | Vista de mapa mundial con sidebar lateral. Layout: ┌──────────────────────────────────────┬──────────────┐ │ │ Sidebar: │ │ MapLibre canvas │ - lista │ │ (clusters verdes, popup al click) │ - search │ │ │ - updates │ └──────────────────────────────────────┴───… |
| `src/components/MedalBadge.tsx` | 167 | TIERS, MedalBadge | (v7) Medalla de un logro: cinta + disco metálico por nivel + icono propio + pips de nivel. El mismo sistema para los 23 logros — el nivel (bronce → diamante) cambia el metal, así que la medalla evoluciona con el progreso. `tier = null` (aún sin desbloquear) → … |
| `src/components/NotificationsBell.tsx` | 752 | NotificationsBell | Campana de **Notificaciones** del header. Antes mostraba sólo actualizaciones de paquetes Community; ahora también incluye: · Nueva versión de la app disponible (top del listado). · Updates de paquetes (como antes). El refresh es automático — el bootstrap del … |
| `src/components/OnboardingTour.tsx` | 431 | OnboardingTour | Tour de bienvenida guiado — recorre las 5 pantallas principales de la app + la configuración esencial, **cambiando de pantalla solo a medida que avanza (el usuario no tiene que navegar a mano). Se dispara automáticamente la primera vez que la app arranca y `pr… |
| `src/components/PackageDetailModal.tsx` | 780 | PackageDetailModal, RepairMethodPicker, Detail, GsxProfileRow, formatBytes | Modal con el detalle de un paquete instalado en Community. Tres acciones principales en la fila inferior: 1. **Reparar** — abre un sub-popup con todos los métodos de descarga disponibles (en cada fuente). El usuario elige uno → desinstalamos + descargamos. Est… |
| `src/components/PerfBadge.tsx` | 28 | PerfBadge | (v5.0.0) Micro-badge "optimizable": una llave inglesa AMARILLA que marca los escenarios a los que SÍ se les puede aplicar el optimizador de FPS (tienen "Optional Configuration" en SceneryAddons). Si un aeropuerto no lo lleva, no tiene opciones y su modal queda… |
| `src/components/PerformanceModal.tsx` | 489 | PerformanceModal | Referencia estable para el fallback vacío de `savedIds` — NUNCA crear un `[]` nuevo dentro de un selector de zustand (dispara React #185). |
| `src/components/PilotBadge.tsx` | 217 | PilotBadge, Avatar | (v6.1 #30) Insignia del piloto — vive en el header GLOBAL (a la derecha del botón Downloads) para que el nivel/avatar salga en TODAS las pantallas, no solo en el Hangar. Despliega un menú con foto editable + nivel/XP. El avatar persiste local + nube (solo la f… |
| `src/components/PreflightHost.tsx` | 49 | PreflightHost | (v6.2.34) Anfitrión a nivel App del pre-vuelo: · Renderiza el modal cuando `usePreflightStore.open`. · Vigila el OFP del DÍA: al detectar uno nuevo (id distinto del último avisado), suena una campana, trae la app al frente y abre el modal para que el usuario v… |
| `src/components/PreflightModal.tsx` | 365 | PreflightModal | (v6.2.26 / R3) Modal "¿Listo para volar?". Dos secciones: 1. Checklist pre-vuelo — cruza el próximo vuelo (último OFP de SimBrief o el origen/destino en vivo del sim) con el escenario de salida/llegada instalado, su estado activo/update, y el AIRAC. Cada punto… |
| `src/components/RecordingSettings.tsx` | 252 | RecordingSettings | (v6 #2b) Ajustes de "Best Landings" (grabación). Motor: windows-record (audio del sistema nativo + captura DX/fullscreen + replay buffer), integrado en la app — sin descargas. Graba la ventana de MSFS directamente, así que no hay selector de monitor: opciones … |
| `src/components/RegionBadge.tsx` | 36 | RegionBadge | (v5.0.0) Micro-badge de región/país a partir del ICAO. Color por continente. Compartido por la card del Map, la sidebar y los nodos de aeropuerto del Link Map para que el usuario identifique de un vistazo a qué región pertenece cada aeropuerto. |
| `src/components/ReplayBanner.tsx` | 87 | ReplayBanner | (v4.0.0 P7.7) Banner top-right que aparece cuando el watcher detecta modo Replay o Slew activo en el simulador. Backend emite `flight://replay` con payload: { active: bool, simRate: number, isSlew: bool } Mientras está activo, el watcher CONGELA toda la lógica… |
| `src/components/ResultCard.tsx` | 600 | ResultCard | Estado de instalación de un addon de búsqueda respecto a lo que el usuario ya tiene en Community. Se computa correlando por ICAO (el match más fiable que tenemos) entre el resultado del catálogo y la tabla `community_packages`. · "no-installed" → no aparece en… |
| `src/components/ResultsList.tsx` | 93 | ResultsList | Cuando es "browse", los resultados son del catálogo paginado (sin query). Cambia los textos vacíos para no decir "sin resultados para ''". |
| `src/components/RoutesMapView.tsx` | 2156 | RoutesMapView | (v4.18.1 → v4.21.0) Bandas del terminator día/noche. Cada banda es el polígono donde el sol está por debajo de `altitudeDeg`; al apilarlas la oscuridad crece hacia la noche cerrada (-18°, crepúsculo astronómico). v4.21.0: opacidades MÁS marcadas — con 0.15 el … |
| `src/components/SearchBar.tsx` | 80 | SearchBar |  |
| `src/components/SettingsModal.tsx` | 2298 | SettingsModal, LanguageRow, RestartHintModal | Modal de configuración. Layout: una sola columna con secciones (General, Vuelos, Mapa, Carpetas, Almacenamiento, Acerca de). Toggles aplican al instante — sin botón "Guardar". Errores se renderizan al inicio del modal. Vive globalmente en `App.tsx` para flotar… |
| `src/components/ShareCardModal.tsx` | 141 | ShareCardModal | (v6.2.30 / R7) Shell reutilizable para mostrar una tarjeta SVG compartible (vuelo o Wrapped) con acciones de copiar al portapapeles y guardar como PNG. La rasterización vive en lib/shareImage. |
| `src/components/SimBriefConfirmModal.tsx` | 263 | SimBriefConfirmModal | (v4.17.0) Modal de confirmación de OFP al APAGADO DE MOTORES — solución para cuentas SimBrief COMPARTIDAS, donde la API devuelve el último plan generado por cualquiera de los usuarios y el auto-link cruzaba callsigns. Flujo: 1. El watcher (populate_simbrief_as… |
| `src/components/SimVersionModal.tsx` | 96 | SimVersionModal | (v5.1.0) Modal de elección de versión de MSFS al primer arranque. Bloqueante: el usuario elige 2020 o 2024 y la app aplica el filtro del catálogo de SceneryAddons + la carpeta Community de esa versión. Sólo aparece cuando `settings.simVersion` está vacío. (v5.… |
| `src/components/SkyboundSettings.tsx` | 76 | SkyboundSettings | (v6.2.38) Credenciales de Skybound (catálogo MSFS2024). El usuario las guarda aquí; el backend mantiene la sesión. Sólo es útil en MSFS2024. |
| `src/components/SourceToggle.tsx` | 48 | SourceToggle |  |
| `src/components/SplashScreen.tsx` | 370 | SplashScreen, SplashTask | Splash screen — pantalla de carga inicial. Diseño: minimal, profesional, undecorated (sin botones de cerrar/minimizar — la idea es que el usuario NO interactúe mientras carga). Estados especiales: · `appUpdate` populado → muestra un banner "Hay v0.X.Y" con bot… |
| `src/components/TitleBar.tsx` | 124 | TitleBar | Custom titlebar dark — reemplaza al titlebar nativo de Windows. La ventana corre con `decorations: false` en `tauri.conf.json`, por eso pintamos nuestro propio header con: · `data-tauri-drag-region` para arrastrar la ventana. · Logo + nombre alineados a la izq… |
| `src/components/Toaster.tsx` | 92 | Toaster | Renderizador global de toasts. Pinta una columna en la esquina inferior derecha. Cada toast entra deslizándose desde la derecha y sale fundiéndose. `framer-motion` se encarga del enter/exit automáticamente — sólo enchufamos el array del store. |
| `src/components/UpdateBanner.tsx` | 316 | UpdateBanner | Banner amarillo en la parte superior de la app que avisa cuando GitHub Releases publicó una versión mayor a la instalada. Se muestra una sola vez por versión: si el usuario lo cierra, queda registrado en `localStorage` con la versión dismissed para no repetir … |
| `src/components/UpdateWizard.tsx` | 341 | UpdateWizard | Wizard global que itera la cola `updateQueue` del community store y muestra para cada item los métodos de descarga disponibles. El usuario: · Elige un método → desinstalamos el paquete actual + lanzamos la descarga (que reinstala). Pasa al siguiente. · Salta →… |
| `src/components/VsCombatModal.tsx` | 368 | VsCombatModal | ICAO de aerolínea desde el callsign (primeras 3 letras). |
| `src/components/WhatsNewModal.tsx` | 278 | isWhatsNewPending, WHATS_NEW_CLOSED_EVENT, WhatsNewModal | (v7) "Novedades" — modal BLOQUEANTE que se muestra UNA vez por versión al arrancar. A diferencia del carrusel viejo: NO tiene X ni se cierra al hacer clic afuera ni con Esc — el usuario DEBE leerlo. Las novedades se reparten en páginas de 4 (paginado dinámico)… |

## Apéndice F — Stores Zustand (`src/stores`)

| Archivo | Líneas | Exporta | Descripción |
|---|---:|---|---|
| `src/stores/useAchievementsStore.ts` | 40 | useAchievementsStore | (v7) Logros. Se calculan en el backend a partir del historial de vuelos (`flight_log`), así que no hay estado persistente aquí: sólo el último resultado + un refresh bajo demanda (al abrir el panel) y tras cada vuelo. |
| `src/stores/useAiracUpdateStore.ts` | 83 | useAiracUpdateStore, airacUpdateVisible | (v6.2.8) Estado de actualización del AIRAC (datos de navegación). El backend `airac_check_update` calcula el ciclo vigente del calendario fijo de 28 días y lo compara con el instalado. La app consulta al arrancar y cada hora; se comparte aquí para Notificacion… |
| `src/stores/useAppStore.ts` | 255 | View, useAppStore | Vista activa en `App`. Dashboard (default home, totales) / Buscar / Mapa (sólo escenarios) / Addons (resto: aircraft, livery, sound, etc) / FlightBook (historial de vuelos reales con métricas de SimConnect). |
| `src/stores/useCommunityStore.ts` | 244 | useCommunityStore | Estado compartido entre la vista de mapa y la campana de notificaciones del header. Mantiene en memoria: · El inventario de paquetes en Community (lo que el scanner leyó la última vez). · La lista de updates detectados (ya filtrada de "dismissed"). · La cola d… |
| `src/stores/useDownloadsStore.ts` | 327 | PanelTab, DownloadRecovery, useDownloadsStore, toJobsList | Pestañas dentro del panel lateral. La de descargas activas es la que ya existía; «installed» es la nueva pestaña con el historial persistente leído desde `installed_addons`. |
| `src/stores/useFlightLogStore.ts` | 324 | useFlightLogStore | (v4.0.0 — P1 fix flicker) Merger inteligente para `FlightStatus` que viene del watcher (evento `flight://current`). Por qué existe**: el watcher tiene dos paths que emiten ese evento: 1. **SimConnect FFI thread** — 4Hz, full state (lat/lon/heading/ phase/airpo… |
| `src/stores/useFlightsimUpdateStore.ts` | 52 | useFlightsimUpdateStore | (v6.2.60) Updates de addons descargados de flightsim.to por el navegador embebido. Forward-only y FIABLE: al instalar guardamos el `file_id` + `updatedAt` (baseline) en el backend; aquí sólo pedimos el resultado del chequeo (compara el updatedAt actual contra … |
| `src/stores/useFpsAppliedStore.ts` | 60 | useFpsAppliedStore | (v6.2.25) Recuerda qué optimizaciones FPS (ids de opción) tenías APLICADAS por escenario (folderName). Cuando un update reinstala el escenario y borra los renombrados de .bgl, esta memoria permite ofrecer "restaurar como estaba" en un clic — el .bgl vuelve a d… |
| `src/stores/useFpsNoticesStore.ts` | 184 | FpsNotice, useFpsNoticesStore | (v6.2.20 · v6.2.23) Avisos de "FPS Optimization" para la campanita: · escenario NUEVO instalado con objetos opcionales → aviso; · escenario ACTUALIZADO (el update lo reinstala y BORRA las optimizaciones aplicadas) → aviso de re-optimizar. El clic abre el modal… |
| `src/stores/useGsxLocalStore.ts` | 75 | useGsxLocalStore | (v1.1.4) Store mínimo para la lista de ICAOs con perfil GSX instalado localmente en `%APPDATA%\Virtuali\GSX\MSFS`. La consume `ResultCard` y `PackageDetailModal` para mostrar un check al lado del ICAO del escenario cuando el usuario ya tiene su perfil GSX matc… |
| `src/stores/useGsxOfferStore.ts` | 171 | GsxOffer, useGsxOfferStore | (v6.2.64) Oferta de perfiles GSX al instalar un escenario de aeropuerto. Cuando aparece una carpeta NUEVA en Community con ICAO (un escenario recién instalado), consultamos flightsim.to por perfiles GSX de ese aeropuerto y, si hay, mostramos un modal con nombr… |
| `src/stores/useGsxUpdateStore.ts` | 133 | useGsxUpdateStore, gsxUpdateVisible | (v6.2.4) Estado de actualización de GSX (FSDreamTeam couatl). El backend `gsx_check_update` parsea las notas de FSDT y compara con la versión instalada. La app consulta cada hora; el resultado se comparte aquí para pintarlo en Notificaciones y en el Dashboard.… |
| `src/stores/useLiveVsStore.ts` | 614 | VsOfp, VsPilot, VsPos, VsLanding, VsAircraft, vsChannelKey, loadVsSnapshot, findVsSnapshot… | (v6 #3) Live VS — estado del modo competitivo Daniel vs Héctor sobre Supabase Realtime. Modelo: ambos pilotos, volando la MISMA ruta el MISMO día, se unen al mismo canal `simfleet-vs-<fecha>-<origen>-<destino>`. Por **presence** cada cliente publica su identid… |
| `src/stores/useMaintAlertsStore.ts` | 89 | useMaintAlertsStore | (v6.2.19) Alertas de mantenimiento para la campanita: aviones con algún componente >=75% de desgaste. Se refrescan al arrancar y cada vez que el flight log cambia (aterrizaste → el desgaste subió). Descartes: por matrícula+componente en localStorage, guardando… |
| `src/stores/useNewSceneryStore.ts` | 142 | useNewSceneryStore | (v6.2.26 / R3) Radar de escenarios NUEVOS del catálogo. Al arrancar (y bajo demanda) navega la primera página de SceneryAddons y compara las fechas de publicación con un marcador local: los addons publicados DESPUÉS de la última vez que el usuario "vio las nov… |
| `src/stores/usePerfStore.ts` | 101 | usePerfStore | (v5.0.0) Store del estado "optimizable" de los escenarios: qué aeropuertos tienen objetos opcionales desactivables (autos, pasajeros, GSE, clutter…) y por tanto pueden mostrar el badge de la tuerca + abrir el modal de rendimiento. Se llena bajo demanda con `en… |
| `src/stores/usePreflightStore.ts` | 69 | usePreflightStore | (v6.2.34) Estado del pre-vuelo "¿Listo para volar?". · `open` — el modal se muestra a nivel App (para poder auto-abrirlo al detectar el OFP del día desde cualquier pantalla). · `bypass` — ICAOs marcados como "usaré el escenario por defecto"; persistido, para q… |
| `src/stores/useSettingsStore.ts` | 355 | useSettingsStore | (v3.39.0 #3) Campos de unidad en `AppSettings` (clave del store). |
| `src/stores/useSimBriefStore.ts` | 95 | useSimBriefStore | Estado SimBrief: pilotId configurado + historial de vuelos persistido en SQLite + flags de carga. El bootstrap se llama una vez al montar `App`. Si hay pilotId configurado, también dispara un refresh en background para que el último OFP esté disponible al inst… |
| `src/stores/useToastStore.ts` | 82 | ToastKind, Toast, useToastStore | Notificaciones efímeras (toasts) para feedback inmediato: · "info" — descarga arrancada, manual ejecutándose… · "success" — instalación completada con éxito. · "error" — fallo de descarga o instalación. El componente `<Toaster />` montado globalmente en `App` … |

## Apéndice G — Librerías del frontend (`src/lib`) y raíz (`src/`)

| Archivo | Líneas | Exporta | Descripción |
|---|---:|---|---|
| `src/lib/aircraft.ts` | 49 | cleanAtcType | (v5.2.9 / v5.3.0) Normaliza el `ATC TYPE` de SimConnect para mostrarlo en el FlightBook, funcionando en MSFS 2020 **y** 2024. - **MSFS 2020**: la SimVar `ATC TYPE` ya viene resuelta (ej. "Boeing", "Airbus") → la devolvemos tal cual. - **MSFS 2024**: devuelve e… |
| `src/lib/airlineCodes.ts` | 584 | ICAO_TO_IATA, icaoToIata, nameToIata, resolveIata, icaoToName, canonicalAirlineKey | (v4.9.0) Mapa ICAO (3 letras) → IATA (2 letras) de aerolíneas. Guardamos el código ICAO por vuelo (callsign/OFP), pero los CDN de logos gratuitos (avs.io, daisycon) indexan por IATA. Este mapa CURADO cubre las aerolíneas grandes + carriers modernos que faltan … |
| `src/lib/airlineCodesData.ts` | 17 | OF_ICAO_TO_IATA, OF_CALLSIGN_TO_IATA, OF_NAME_TO_IATA, OF_ICAO_TO_NAME |  |
| `src/lib/avatarStore.ts` | 134 | subscribeAvatar, loadAvatar, saveAvatar | (v6.1 #30) Persistencia del avatar del piloto: LOCAL (localStorage) + NUBE (Supabase, SOLO la foto). Por orden del usuario activamos la nube únicamente para el avatar — NO se suben puntos ni vuelos (el sync general sigue inactivo). Tabla esperada en Supabase (… |
| `src/lib/cn.ts` | 4 | cn |  |
| `src/lib/displayTitle.ts` | 65 | disambiguateTitles | (v4.26.0) Desambiguación de títulos duplicados de addons. Varios paquetes pueden declarar el MISMO title en su manifest: · `pmdg-aircraft-738` y `pmdg-tablet-73x` → ambos "737-800" (el segundo es el EFB/tablet de PMDG, no el avión). · Liveries sueltas titulada… |
| `src/lib/flightTelemetry.ts` | 158 | engineCount, engineModel, TelemetryPoint, FlightTelemetry, deriveTelemetry | (v6.1) Telemetría SINTÉTICA por vuelo para el detalle desplegable del Hangar. El flight_log guarda agregados reales (altitud máx, velocidad máx, distancia, combustible, FPM) pero NO series de tiempo de motores. Aquí derivamos una curva de EGT/N1 CREÍBLE y zona… |
| `src/lib/flightbookExport.ts` | 191 | FlightBookExportLabels, buildFlightBookHtml, saveFlightBookHtml | (v6.2.31 / R8) Exporta el FlightBook a un archivo HTML AUTOCONTENIDO y de SOLO LECTURA: se abre en cualquier navegador sin conexión ni SimFleet instalado. Incluye CSS inline, un mapa de rutas SVG en proyección equirectangular (sin depender de tiles online) y l… |
| `src/lib/greatCircle.ts` | 72 | greatCircleLine | Genera un polyline de N segmentos que aproxima el arco del círculo máximo entre dos puntos de la Tierra. Usar esto en MapLibre hace que las líneas SimBrief/SimConnect se vean curvadas como en una proyección de globo terráqueo (en vez de líneas rectas Mercator … |
| `src/lib/hangarMaintenance.ts` | 224 | GearComponent, ReportPart, MaintenanceReport, MaintenanceData, deriveMaintenance | (v6.1 #31 #32) Mantenimiento SINTÉTICO del Hangar. No hay tracking real de mantenimiento; derivamos un historial y un estado de componentes CREÍBLES a partir del uso real del avión (horas, ciclos = vuelos, aterrizajes duros, salud del tren). Todo es DETERMINIS… |
| `src/lib/i18n.ts` | 122 | Locale, LocaleSetting, resolveLocale, setActiveLocale, getActiveLocale, preloadLocale, persistLocale, t | (v3.4.0 → v3.5.0) Sistema i18n centralizado. Arquitectura: · Diccionarios **en JSON files** — `i18n/es.json` y `i18n/en.json`. Importados con la magia de Vite/TS para `resolveJsonModule`, emitidos como object literal en el bundle. · `t(key, args?)` hace lookup… |
| `src/lib/landingSound.ts` | 139 | playLandingSound | Chirrido resonante de las llantas (con vibrato + barrido de frecuencia). |
| `src/lib/liveVsConfig.ts` | 28 | DEFAULT_VS_SUPABASE_URL, DEFAULT_VS_SUPABASE_KEY, normalizeSupabaseUrl | (v6 #3) Credenciales por defecto de Live VS (Supabase Realtime). La `anon public` key es pública por diseño (pensada para ir en clientes; protegida por RLS y, como Live VS no usa NINGUNA tabla — solo presence + broadcast de Realtime — no expone datos). Por eso… |
| `src/lib/liveryslugs.ts` | 263 | LiveryTarget, liverySlugFor, liveryTargetsFor, liverySearchTermFor, liveryUrlFor | (v7.2.5) Resolución de la página de liveries de flightsim.to para CUALQUIER avión — siempre devuelve una URL útil. DOS niveles: 1. Página dedicada `https://flightsim.to/liveries/<slug>` para los aviones conocidos (tabla RESOLVERS de slugs VERIFICADOS contra la… |
| `src/lib/nativeNotify.ts` | 93 | nativeNotificationsEnabled, setNativeNotificationsEnabled, NativeNotifyOptions, nativeNotify | (v7.4.0) Notificaciones NATIVAS del sistema operativo (el Action Center de Windows), separadas del `<Toaster/>` in-app. Se disparan sobre todo cuando la ventana está en SEGUNDO PLANO: SimFleet suele correr minimizado a la bandeja mientras el usuario vuela, así… |
| `src/lib/oaciRegion.ts` | 321 | Continent, AirportRegion, airportRegion, continentLabel, regionBadgeClass, oaciRegionLabel | (v5.0.0) Identificadores regionales a partir del código ICAO. El primer carácter del ICAO fija el "continente/zona" OACI; los dos primeros suelen fijar el PAÍS (LE=España, MD=Rep. Dominicana, KMIA=EE.UU., …). Con eso pintamos un micro-badge de color en cada ca… |
| `src/lib/packageType.ts` | 284 | DerivedType, diagnosePackage, derivedType, is3rdParty, looksLikePlaceholderTitle, isScenery, isSdkSamplePackage, isAirport… | Tipo derivado del paquete a partir del manifest. Usamos esto en lugar del `contentType` crudo porque MSFS no distingue liveries de aircraft completos — ambos son `AIRCRAFT`. Para separarlos miramos señales explícitas (nombre, keywords, content_type). Mantener … |
| `src/lib/preflight.ts` | 325 | CheckStatus, PreflightAction, PreflightCheck, PreflightRoute, PreflightInput, pickTodayPlan, PreflightResult, computePreflight | (v6.2.26 / R3) Lógica pura del chequeo pre-vuelo "¿Listo para volar?". Cruza el próximo vuelo (último OFP de SimBrief, o el origen/destino en vivo del sim) con lo que el usuario tiene instalado y al día: · escenario de salida/llegada instalado, activo y sin up… |
| `src/lib/shareCards.ts` | 289 | BuiltCard, FlightCardLabels, buildFlightCardSvg, WrappedLabels, buildWrappedSvg | (v6.2.30 / R7) Generadores de tarjetas SVG compartibles: una para un vuelo concreto y otra para el "Wrapped" anual. Autocontenidas (sólo vector + texto, fuente del sistema) para poder rasterizarlas a PNG en un canvas sin "tainting". Los textos ya vienen traduc… |
| `src/lib/shareImage.ts` | 93 | svgToPngBlob, copyImageToClipboard, SaveResult, savePngToDisk | (v6.2.30 / R7) Utilidades para exportar una tarjeta SVG como imagen. Rasterizamos el SVG (autocontenido: vector + texto, sin imágenes externas para evitar canvas "tainted") a PNG en un canvas y luego: · lo copiamos al portapapeles (compartir pegando en Discord… |
| `src/lib/smooth.ts` | 262 | LngLat, LngLatTs, segmentTrackCoords, smoothCatmullRom | (v2.2.0) Suavizado de polylines con Catmull-Rom spline. El watcher de SimConnect samplea posición cada 10 s. A 250 kt eso son ~1.3 km entre cada punto, lo cual al renderizar como LineString muestra giros muy poligonales (especialmente en approaches y patrones … |
| `src/lib/sound.ts` | 36 | playPreflightChime | (v6.2.34) Sonido de aviso corto vía Web Audio — sin assets. Se usa al detectar el OFP del día para llamar la atención del usuario junto con el foco de la ventana. Dos tonos ascendentes tipo "campana". |
| `src/lib/tauri.ts` | 2580 | isTauri, DiagProcInfo, DiagProcReport, MaintAlert, DiagAppMetrics, api | True when we're running inside the Tauri webview (vs. a plain browser). |
| `src/lib/terminator.ts` | 165 | buildTerminatorPolygon | (v4.0.0 — P6 · v7.4.2) Terminator día/noche del RoutesMapView. Computa el polígono GeoJSON de la zona donde el sol está por debajo de `altitudeDeg` (0° = noche geométrica, -6/-12/-18° = crepúsculos) para un instante UTC. Pintado como `fill-layer` sobre el glob… |
| `src/lib/testEnv.ts` | 34 | isTestEnv, supabaseTable, vsChannelPrefix | (v7.4.2) ENTORNO DE PRUEBAS — lado frontend. El backend (instancia de pruebas, ver `src-tauri/src/test_env.rs`) inyecta `window.__SIMFLEET_TEST_ENV__ = true` en el webview ANTES de cargar la página, así que esto se sabe de forma síncrona desde el primer render… |
| `src/lib/thumbnails.ts` | 79 | clearThumbnailCache, useThumbnail | (v4.25.0) Hook compartido de thumbnails de paquetes Community. Antes vivía duplicado dentro de AddonsView; ahora lo consumen las cards del grid, los nodos del Link Map y el popover del mapa. El cache es a nivel de módulo (sobrevive a remounts) y guarda `null` … |
| `src/lib/types.ts` | 1530 | DownloadKind, DownloadMethod, Addon, SourceDescriptor, BrowsePage, DownloadPhase, DownloadJob, CommunityInfo… | (v7) Partes ADICIONALES de un archivo multi-parte (Skybound). Cuando está presente, `url` es la parte 1 y el resto se baja/une en el backend. El frontend sólo lo transporta de vuelta a `start_download`. |
| `src/lib/units.ts` | 182 | Units, makeUnits, useUnits | (v3.28.0 P7.11 · v3.39.0 #3) Sistema de unidades — POR CATEGORÍA. Toda la telemetría se ALMACENA en unidades canónicas (las que entrega SimConnect / la DB): distancia=nm, altitud=ft, velocidad=kt, vert.speed=fpm, peso=kg, temperatura=°C, presión=hPa, flujo=pph… |
| `src/lib/unitsStorage.ts` | 103 | WeightUnit, AltitudeUnit, SpeedUnit, VsUnit, DistanceUnit, PressureUnit, TempUnit, UnitSystem… | (v3.28.0 P7.11 · v3.39.0 #3) Espejo de las preferencias de unidades en localStorage, lectura SÍNCRONA pre-render (igual que i18n). (v3.39.0 #3) Unidades POR CATEGORÍA: cada magnitud (peso, altitud, velocidad, V/S, distancia, presión, temperatura) se elige por … |
| `src/lib/updateSearchQuery.ts` | 57 | looksLikeIcao, deriveUpdateSearchQuery | Deriva un query de búsqueda razonable a partir de una update disponible. La rama AIRCRAFT del SQL en `catalog_versions_for_community` emite `icao = ""` para paquetes no-SCENERY (aviones, liveries, sounds), así que usar `u.icao` directamente como query produce … |
| `src/lib/usePreflight.ts` | 47 | usePreflight | (v6.2.34) Hook compartido del pre-vuelo: cruza el OFP del DÍA con lo instalado/al día + bypass del usuario. Lo usan el Dashboard (badge), la campanita (aviso persistente) y el modal, para que todos coincidan. |
| `src/lib/vendorSynonyms.ts` | 31 | expandVendorQuery | (v4.33.0) Sinónimos de vendor para la búsqueda de addons. El usuario escribe "fenix" pero los folders/manifests usan "fnx"; "flybywire" vs "fbw"; etc. Sin esto, buscar "fenix" sólo matchea el avión base (title "Fenix Airbus A320") y NO sus liveries (folder "fn… |
| `src/lib/version.ts` | 87 | compareVersions, isNewer | Comparación de versiones semver tolerante. Réplica del helper de `updates.rs` en backend para que el frontend pueda decidir "¿esta versión del catálogo es más nueva que la instalada?" sin un round-trip al backend. Reglas: · Acepta `v1.2.3`, `1.2.3`, `1.2`, `1`… |
| `src/lib/wrapped.ts` | 111 | WrappedTop, WrappedStats, computeWrapped | (v6.2.30 / R7) Estadísticas anuales estilo "Wrapped" derivadas del FlightBook. Puro y testeable — sólo agrega sobre los vuelos completados del año pedido. |

| Archivo | Líneas | Exporta | Descripción |
|---|---:|---|---|
| `src/App.tsx` | 1162 | App | Bootstrap centralizado: una sola tarea async awaita Promise.all de los bootstraps de cada store. La splash screen permanece hasta que todos terminan (o fallan, marcados con error). El app principal sólo se monta cuando `ready === true`. |
| `src/main.tsx` | 51 |  |  |

## Apéndice H — Esquema SQLite (DB totalmente migrada)

23 tablas, 45 índices explícitos, 0 triggers, 0 vistas. Migraciones: `src-tauri/migrations/` (sqlx, embebidas con `sqlx::migrate!`).

- **`addon_link_positions`** — folder_name TEXT PK, x REAL NN, y REAL NN
- **`addon_links`** — id INTEGER PK, source_folder TEXT NN, target_folder TEXT NN, origin TEXT NN, created_at TEXT NN  · índices: idx_addon_links_source, idx_addon_links_target
- **`addons`** — id TEXT PK, source TEXT NN, title TEXT NN, developer TEXT, name TEXT NN, version TEXT, icao TEXT, simulator TEXT, page_url TEXT NN, first_seen_at TEXT NN, last_seen_at TEXT NN, released_at TEXT  · índices: idx_addons_developer, idx_addons_icao, idx_addons_released_at, idx_addons_source
- **`airline_economy`** — airline_key TEXT PK, airline_icao TEXT, airline_name TEXT, is_lowcost INTEGER NN, start_valuation REAL NN, revenue REAL NN, costs REAL NN, flights INTEGER NN, updated_at TEXT, passengers INTEGER NN, revenue_tickets REAL NN, revenue_ancillary REAL NN, cost_fuel REAL NN, cost_catering REAL NN, cost_handling REAL NN, cost_maintenance REAL NN, cost_fees REAL NN, avg_landing_fpm REAL, satisfaction REAL, gsx_flights INTEGER NN, is_cargo INTEGER NN, fleet_size INTEGER NN, fleet_value REAL NN
- **`airline_policy`** — airline_key TEXT PK, maintenance_level INTEGER NN, snacks INTEGER NN, meals INTEGER NN, wifi INTEGER NN, seat_upgrades INTEGER NN, priority_boarding INTEGER NN, extra_baggage INTEGER NN, updated_at TEXT
- **`airports`** — icao TEXT PK, name TEXT NN, latitude REAL NN, longitude REAL NN, iso_country TEXT, type TEXT, municipality TEXT  · índices: idx_airports_country, idx_airports_type
- **`community_packages`** — folder_name TEXT PK, install_path TEXT NN, title TEXT NN, creator TEXT, content_type TEXT, package_version TEXT, minimum_game_version TEXT, icao TEXT, size_bytes INTEGER, folder_modified_at TEXT, scanned_at TEXT NN, dependencies_count INTEGER NN, enabled INTEGER NN, simobject_dirs_json TEXT NN, base_containers_json TEXT NN, thumbnail_path TEXT, has_own_model INTEGER NN, has_manifest INTEGER NN, has_flight_model INTEGER NN  · índices: idx_community_packages_content_type, idx_community_packages_creator, idx_community_packages_icao
- **`dismissed_updates`** — folder_name TEXT PK, dismissed_at TEXT NN, latest_version TEXT NN
- **`download_jobs`** — id TEXT PK, addon_id TEXT NN, addon_title TEXT NN, source TEXT NN, method_kind TEXT NN, method_name TEXT NN, url TEXT NN, phase TEXT NN, bytes_total INTEGER NN, bytes_done INTEGER NN, speed_bps INTEGER NN, eta_seconds INTEGER, message TEXT, error TEXT, install_path TEXT, created_at TEXT NN, updated_at TEXT NN  · índices: idx_download_jobs_created_at, idx_download_jobs_phase
- **`downloads`** — id INTEGER PK, addon_id TEXT, url TEXT NN, method TEXT NN, status TEXT NN, bytes_total INTEGER, bytes_done INTEGER, started_at TEXT NN, finished_at TEXT, error TEXT  · índices: idx_downloads_addon_id, idx_downloads_status
- **`flight_log`** — id INTEGER PK, started_at TEXT NN, ended_at TEXT, origin_lat REAL NN, origin_lon REAL NN, origin_icao TEXT, origin_name TEXT, destination_lat REAL, destination_lon REAL, destination_icao TEXT, destination_name TEXT, aircraft_title TEXT, aircraft_atc_type TEXT, distance_nm REAL, flight_time_s INTEGER, max_altitude_ft INTEGER, source TEXT NN, landing_fpm INTEGER, max_ground_speed_kt INTEGER, max_true_airspeed_kt INTEGER, departure_gate TEXT, arrival_gate TEXT, last_position_lat REAL, last_position_lon REAL, last_position_alt_ft INTEGER, last_position_gs_kt INTEGER, last_position_at TEXT, passengers INTEGER, cargo_kg INTEGER, fuel_used_kg INTEGER, paused_seconds INTEGER NN, aircraft_model TEXT, aircraft_airline TEXT, aircraft_registration TEXT, external_id TEXT, flight_number TEXT, callsign TEXT, airline_icao TEXT, status TEXT NN, score_total INTEGER, score_max INTEGER, score_grade TEXT, simbrief_ofp_id TEXT, bounced INTEGER NN, max_brake_temp_c REAL, route_fixes TEXT, metar_origin TEXT, metar_dest TEXT, partial_ack INTEGER NN, max_tire_wear_pct REAL  · índices: idx_flight_log_airline_icao, idx_flight_log_destination_icao, idx_flight_log_ended, idx_flight_log_external, idx_flight_log_flight_number, idx_flight_log_origin_icao, idx_flight_log_simbrief_ofp_id, idx_flight_log_started, idx_flight_log_status, uq_flight_log_va_dedup_completed
- **`flight_log_phase`** — id INTEGER PK, flight_id INTEGER NN, phase TEXT NN, entered_at TEXT NN, exited_at TEXT  · índices: idx_phase_flight, idx_phase_flight_entered, uq_phase_flight_phase_entered
- **`flight_log_score_item`** — id INTEGER PK, flight_id INTEGER NN, phase TEXT NN, rule_id TEXT NN, label TEXT NN, points_earned INTEGER NN, points_max INTEGER NN, passed INTEGER NN, severity TEXT, evidence TEXT, ts TEXT  · índices: idx_score_item_flight, idx_score_item_flight_phase, uq_score_item_flight_rule
- **`flight_log_track`** — id INTEGER PK, flight_id INTEGER NN, lat REAL NN, lon REAL NN, alt_ft INTEGER, gs_kt INTEGER, ts TEXT NN, ias_kt INTEGER, vs_fpm INTEGER, pitch_deg REAL, bank_deg REAL, g_force REAL, flaps_pct INTEGER, gear_down INTEGER, spoilers_pct INTEGER, light_nav INTEGER, light_beacon INTEGER, light_taxi INTEGER, light_landing INTEGER, light_strobe INTEGER, parking_brake INTEGER, transponder_code INTEGER, eng_n1_1 REAL, eng_n1_2 REAL, eng_n1_3 REAL, eng_n1_4 REAL, eng_n2_1 REAL, eng_n2_2 REAL, eng_n2_3 REAL, eng_n2_4 REAL, eng_egt_1 REAL, eng_egt_2 REAL, eng_egt_3 REAL, eng_egt_4 REAL, eng_ff_pph_1 REAL, eng_ff_pph_2 REAL, eng_ff_pph_3 REAL, eng_ff_pph_4 REAL, eng_oil_temp_1 REAL, eng_oil_temp_2 REAL, eng_oil_temp_3 REAL, eng_oil_temp_4 REAL, eng_oil_press_1 REAL, eng_oil_press_2 REAL, eng_oil_press_3 REAL, eng_oil_press_4 REAL, scoring_phase TEXT, wind_dir_deg INTEGER, wind_speed_kt INTEGER, oat_c REAL, baro_hpa INTEGER, visibility_m INTEGER, precip_state INTEGER, cloud_cover_pct INTEGER, cloud_low_pct INTEGER, cloud_mid_pct INTEGER, cloud_high_pct INTEGER, stall_warning INTEGER, overspeed_warning INTEGER, oil_press_warning INTEGER  · índices: idx_flight_log_track_flight_id, idx_flight_log_track_flight_ts, idx_flight_log_track_scoring_phase
- **`flight_pnl`** — flight_id INTEGER PK, airline_key TEXT NN, pax INTEGER, revenue_tickets REAL NN, revenue_ancillary REAL NN, cost_fuel REAL NN, cost_catering REAL NN, cost_handling REAL NN, cost_maintenance REAL NN, cost_fees REAL NN, net REAL NN, gsx_matched INTEGER NN, created_at TEXT  · índices: idx_flight_pnl_airline
- **`flightsim_tracked_files`** — folder_name TEXT PK, file_id TEXT NN, title TEXT, known_version TEXT, known_updated TEXT, link TEXT, tracked_at TEXT NN, last_checked_at TEXT, thumbnail TEXT  · índices: idx_flightsim_tracked_file_id
- **`gsx_lookups`** — icao TEXT PK, profiles_json TEXT NN, fetched_at TEXT NN  · índices: idx_gsx_fetched_at
- **`installed_addons`** — id TEXT PK, addon_id TEXT, source TEXT, title TEXT NN, name TEXT, developer TEXT, version TEXT, install_path TEXT NN, size_bytes INTEGER, installed_at TEXT NN  · índices: idx_installed_addon_id, idx_installed_installed_at
- **`logs`** — id INTEGER PK, ts TEXT NN, level TEXT NN, category TEXT NN, message TEXT NN, data_json TEXT  · índices: idx_logs_level, idx_logs_ts
- **`maintenance_log`** — id INTEGER PK, registration TEXT NN, component TEXT NN, cost REAL NN, serviced_at TEXT NN  · índices: idx_maintenance_log_reg
- **`settings`** — key TEXT PK, value TEXT NN, updated_at TEXT NN
- **`simbrief_flights`** — ofp_id TEXT PK, pilot_id TEXT NN, flight_number TEXT, callsign TEXT, aircraft_icao TEXT, origin_icao TEXT NN, origin_name TEXT, origin_lat REAL NN, origin_lon REAL NN, destination_icao TEXT NN, destination_name TEXT, destination_lat REAL NN, destination_lon REAL NN, route TEXT, distance_nm INTEGER, est_time_enroute_s INTEGER, generated_at TEXT, fetched_at TEXT NN, pax_count INTEGER, cargo_kg INTEGER, fuel_burn_kg INTEGER, units TEXT, aircraft_reg TEXT, plan_ramp_kg INTEGER, route_fixes TEXT  · índices: idx_simbrief_generated, idx_simbrief_pilot
- **`update_check_cache`** — icao TEXT PK, source TEXT PK, last_known_version TEXT, catalog_addon_id TEXT, checked_at TEXT NN  · índices: idx_update_cache_checked_at

Índices ÚNICOS (restricciones de negocio):
- `uq_flight_log_va_dedup_completed` en `flight_log`: `CREATE UNIQUE INDEX uq_flight_log_va_dedup_completed
    ON flight_log(airline_icao, flight_number, date(started_at))
    WHERE airline_icao IS NOT NULL
      AND flight_number IS NOT NULL
      AND status = 'completed'`
- `uq_phase_flight_phase_entered` en `flight_log_phase`: `CREATE UNIQUE INDEX uq_phase_flight_phase_entered
    ON flight_log_phase(flight_id, phase, entered_at)`
- `uq_score_item_flight_rule` en `flight_log_score_item`: `CREATE UNIQUE INDEX uq_score_item_flight_rule
    ON flight_log_score_item(flight_id, phase, rule_id)`

## Apéndice I — Migraciones

63 migraciones:

- `001_initial.sql`
- `002_installed_addons_relax.sql`
- `003_gsx_cache.sql`
- `004_airports.sql`
- `005_community_packages.sql`
- `006_update_check_cache.sql`
- `007_community_dependencies.sql`
- `008_dismissed_updates.sql`
- `009_simbrief.sql`
- `010_flight_log.sql`
- `011_addon_release_date.sql`
- `012_flight_log_metrics.sql`
- `013_flight_log_live_track.sql`
- `014_download_jobs.sql`
- `015_flight_log_track.sql`
- `016_flight_log_extras.sql`
- `017_simbrief_extras.sql`
- `018_flight_log_aircraft_meta.sql`
- `019_flight_log_external_id.sql`
- `020_flight_log_va_fields.sql`
- `021_flight_log_scoring.sql`
- `022_flight_log_track_vas_fields.sql`
- `023_reset_scores_for_new_rubric.sql`
- `024_flight_log_track_engine_metrics.sql`
- `025_flight_log_simbrief_link.sql`
- `026_simbrief_reg_and_ramp_fuel.sql`
- `027_flight_log_bounced.sql`
- `028_flight_log_track_scoring_phase.sql`
- `029_flight_log_track_weather.sql`
- `030_flight_log_track_cloud_cover.sql`
- `031_flight_log_track_stall_warning.sql`
- `032_reset_scores_for_phase_fix.sql`
- `033_dedup_track_unique_index.sql`
- `034_flight_log_track_damage_warnings.sql`
- `035_flight_log_brake_temp.sql`
- `036_reset_scores_landing_taxi_fix.sql`
- `037_reset_scores_predep_engines_fix.sql`
- `038_reset_scores_phase_window_fixes.sql`
- `039_simbrief_route_fixes.sql`
- `040_flight_log_route_fixes.sql`
- `041_reset_scores_evaluation_v415.sql`
- `042_flight_log_metar.sql`
- `043_reset_scores_landing_light_after_10k.sql`
- `044_reset_scores_aviation_rules_v422.sql`
- `045_flight_log_partial_ack.sql`
- `046_addon_enable_links.sql`
- `047_addon_simobjects.sql`
- `048_thumbnail_path.sql`
- `049_aircraft_model_detection.sql`
- `050_reset_scores_parking_brake_v432.sql`
- `051_purge_stale_ofp_cache.sql`
- `052_airline_economy.sql`
- `053_airline_economy_metrics.sql`
- `054_airline_policy.sql`
- `055_airline_cargo.sql`
- `056_maintenance_log.sql`
- `057_airline_fleet.sql`
- `058_flight_tire_wear.sql`
- `059_dismissed_updates_versioned.sql`
- `060_flightsim_tracked_files.sql`
- `061_clear_gsx_lookups_cache.sql`
- `062_flightsim_thumbnail.sql`
- `063_has_flight_model.sql`

## Apéndice J — Claves de la tabla `settings`

Claves detectadas en la tabla `settings` (k/v) desde el backend (heurística; las preferencias de la UI usan el prefijo `pref_`):

- `airports_dataset_fetched_at` — src-tauri/src/airports.rs
- `cloud_folder_sync_last_at` — src-tauri/src/cloud_sync.rs
- `cloud_folder_sync_path` — src-tauri/src/cloud_sync.rs
- `google_client_id` — src-tauri/src/cloud_sync.rs
- `google_client_secret` — src-tauri/src/cloud_sync.rs
- `google_last_sync_at` — src-tauri/src/cloud_sync.rs, src-tauri/src/lib.rs
- `google_refresh_token` — src-tauri/src/cloud_sync.rs, src-tauri/src/lib.rs
- `google_session_expired` — src-tauri/src/cloud_sync.rs
- `google_user_email` — src-tauri/src/cloud_sync.rs, src-tauri/src/flight_log.rs, src-tauri/src/simbrief.rs
- `gsx_installed_version` — src-tauri/src/gsx_update.rs
- `pref_language` — src-tauri/src/commands/livery.rs, src-tauri/src/download/manager.rs
- `pref_minimize_to_tray` — src-tauri/src/lib.rs
- `pref_sim_version` — src-tauri/src/commands/drop.rs, src-tauri/src/lib.rs
- `rec_enabled` — src-tauri/src/landing_recorder.rs
- `rec_fps` — src-tauri/src/landing_recorder.rs
- `rec_max_clips` — src-tauri/src/landing_recorder.rs
- `rec_microphone` — src-tauri/src/landing_recorder.rs
- `rec_osd_enabled` — src-tauri/src/landing_recorder.rs
- `rec_osd_position` — src-tauri/src/landing_recorder.rs
- `rec_output_path` — src-tauri/src/landing_recorder.rs
- `simbrief_pilot_id` — src-tauri/src/simbrief.rs
- `simfleet` — src-tauri/src/discord_rpc.rs

## Apéndice K — Hosts externos usados

| Host | Usado en |
|---|---|
| `accounts.google.com` | src-tauri/src/cloud_sync.rs, src/lib/tauri.ts |
| `api.github.com` | src-tauri/src/updater.rs |
| `api.planespotters.net` | src-tauri/src/planespotters.rs, src/components/FlightBookView.tsx |
| `cancel.simfleet.invalid` | src-tauri/src/commands/livery.rs |
| `console.cloud.google.com` | src-tauri/src/i18n.rs, src/components/SettingsModal.tsx |
| `davidmegginson.github.io` | src-tauri/src/airports.rs |
| `demotiles.maplibre.org` | src/components/MapView.tsx, src/components/RoutesMapView.tsx |
| `discord.com` | src/components/SettingsModal.tsx |
| `download.simplaza.org` | src-tauri/src/sources/simplaza.rs |
| `en.wikipedia.org` | src-tauri/src/airport_thumbs.rs |
| `flagcdn.com` | src/components/HangarView.tsx |
| `flightsim.to` | src-tauri/src/airport_thumbs.rs, src-tauri/src/commands/livery.rs, src-tauri/src/flightsim_track.rs, src-tauri/src/gsx.rs, src/App.tsx, src/components/GsxSearchSummary.tsx, src/components/MapView.tsx, src/components/PreflightModal.tsx, src/lib/liveryslugs.ts, src/lib/tauri.ts |
| `github.com` | src-tauri/src/lib.rs, src-tauri/src/mobiflight_lvars.rs, src-tauri/src/planespotters.rs, src-tauri/src/updater.rs, src/components/SettingsModal.tsx, src/lib/tauri.ts |
| `images.daisycon.io` | src/components/AirlineLogo.tsx |
| `irdsvqxnomatntkwfmax.supabase.co` | src/lib/liveVsConfig.ts |
| `myaccount.google.com` | src/components/SettingsModal.tsx |
| `navigraph.com` | src/stores/useAiracUpdateStore.ts |
| `oauth2.googleapis.com` | src-tauri/src/cloud_sync.rs |
| `pics.avs.io` | src/components/AirlineLogo.tsx, src/components/HangarView.tsx |
| `sceneryaddons.org` | src-tauri/src/sources/sceneryaddons.rs, src/lib/tauri.ts |
| `server.arcgisonline.com` | src/components/IncompleteFlightModal.tsx, src/components/MapView.tsx, src/components/RoutesMapView.tsx |
| `simplaza.org` | src-tauri/src/sources/simplaza.rs, src/lib/tauri.ts |
| `skybound.cx` | src-tauri/src/drop_install.rs, src-tauri/src/install/mod.rs, src-tauri/src/sources/mod.rs, src-tauri/src/sources/skybound.rs |
| `tile.openstreetmap.org` | src/components/MapView.tsx |
| `www.fsdreamteam.com` | src-tauri/src/gsx_update.rs, src/lib/tauri.ts |
| `www.googleapis.com` | src-tauri/src/cloud_sync.rs |
| `www.openstreetmap.org` | src/components/MapView.tsx |
| `www.simbrief.com` | src-tauri/src/simbrief.rs |

## Apéndice L — Variables de entorno

- `SIMFLEET_DATA_DIR` — src-tauri/src/test_env.rs
- `SIMFLEET_DB_COPY` — src-tauri/src/cloud_sync.rs
- `SIMFLEET_DISABLE_WATCHER` — src-tauri/src/commands/community.rs, src-tauri/src/lib.rs, src-tauri/src/test_env.rs
- `SIMFLEET_FORCE_UPDATE_CHECK` — src-tauri/src/commands/updater.rs
- `SIMFLEET_GOOGLE_CLIENT_ID` — src-tauri/src/cloud_sync.rs, src-tauri/src/i18n.rs
- `SIMFLEET_GOOGLE_CLIENT_SECRET` — src-tauri/src/cloud_sync.rs
- `SIMFLEET_SAFE_MODE` — src-tauri/src/lib.rs, src-tauri/src/simconnect_watcher.rs, src-tauri/src/test_env.rs
- `SIMFLEET_SNAPSHOT` — src-tauri/src/cloud_sync.rs
- `WEBVIEW2_USER_DATA_FOLDER` — src-tauri/src/lib.rs

## Apéndice M — Internacionalización

Frontend: `src/lib/i18n/es.json` (1924 claves) y `en.json` (1924 claves); faltan en en: 0, faltan en es: 0. `t(key, args)` en `src/lib/i18n.ts` (placeholders `{nombre}`).

Espacios de nombres (prefijo → nº de claves): `settings` 206, `fb` 206, `region` 181, `hangar` 140, `economy` 112, `whatsnew` 67, `perf` 62, `drop` 59, `addons` 59, `dashboard` 59, `ach` 57, `preflight` 48, `pkg` 44, `rec` 44, `common` 38, `tour` 35, `diag` 34, `downloads` 26, `hangarMaintenance` 26, `notifications` 25, `cleanup` 22, `linkmap` 22, `editflight` 20, `map` 20, `import` 17, `splash` 16, `vs` 16, `share` 15, `result` 14, `fbexport` 14, `flying` 13, `update` 12, `simver` 12, `livery` 11, `sbconfirm` 11, `liveUpdate` 11, `wrapped` 10, `crosslink` 10, `updateBanner` 10, `nav` 8, `palette` 8, `wizard` 8, `edit` 8, `flightTelemetry` 8, `gsxoffer` 7, `changelog` 7, `osd` 7, `results` 6, `dl` 5, `search` 4, `toast` 4, `find` 4, `restart` 3, `replay` 3, `gsx` 3, `titlebar` 3, `notificationsBell` 3, `errorBoundary` 3, `safeMode` 3, `install` 2, `installer` 2, `header` 2, `gsxSearch` 2, `liveVs` 2, `flightsimto` 1, `source` 1, `demo` 1, `flight` 1, `flightLog` 1

Backend: `src-tauri/src/i18n.rs` — tabla `(clave, es, en)` con 250 entradas; macro `tr!("clave", arg = valor)`; el frontend empuja el idioma con `set_backend_locale`.

## Apéndice N — Dependencias y scripts

**npm** (package.json):

- dependencies: `@dagrejs/dagre@^3.0.0`, `@supabase/supabase-js@^2.108.2`, `@tauri-apps/api@^2.1.1`, `@tauri-apps/plugin-dialog@^2.0.1`, `@tauri-apps/plugin-notification@^2.0.0`, `@tauri-apps/plugin-opener@^2.5.3`, `@tauri-apps/plugin-shell@^2.0.1`, `@xyflow/react@^12.11.0`, `clsx@^2.1.1`, `framer-motion@^11.11.17`, `lucide-react@^0.460.0`, `maplibre-gl@^5.24.0`, `react@^18.3.1`, `react-dom@^18.3.1`, `recharts@^3.8.1`, `suncalc@^1.9.0`, `zustand@^5.0.1`
- devDependencies: `@tauri-apps/cli@^2.1.0`, `@types/node@^25.6.0`, `@types/react@^18.3.12`, `@types/react-dom@^18.3.1`, `@types/suncalc@^1.9.2`, `@vitejs/plugin-react@^4.3.3`, `autoprefixer@^10.4.20`, `postcss@^8.4.49`, `tailwindcss@^3.4.15`, `typescript@^5.6.3`, `vite@^5.4.11`
- scripts: `start` = `tauri dev`, `dev` = `vite`, `dev:safe` = `set SIMFLEET_SAFE_MODE=1&& tauri dev`, `build` = `tsc -b && vite build`, `preview` = `vite preview`, `tauri` = `tauri`, `tauri:build` = `tauri build`

**Cargo** (src-tauri/Cargo.toml): `name` simfleet, `version` 7.4.1, `description` SimFleet â€” flight companion for MSFS: addons, FlightBook, SimBrief, GSX, `edition` 2021, `rust-version` 1.77, `name` msfs_addons_browser_lib, `tauri-build` 2, `tauri` 2, `tauri-plugin-shell` 2, `tauri-plugin-opener` 2, `tauri-plugin-dialog` 2, `tauri-plugin-autostart` 2, `tauri-plugin-single-instance` 2, `tauri-plugin-notification` 2, `serde` 1, `serde_json` 1, `tokio` 1, `async-trait` 0.1, `futures-util` 0.3, `reqwest` 0.12, `scraper` 0.20, `url` 2, `urlencoding` 2, `uuid` 1, `zip` 2, `flate2` 1, `walkdir` 2, `unrar` 0.5, `sevenz-rust2` 0.13, `tempfile` 3, `librqbit` 8, `tokio-util` 0.7, `sqlx` 0.8, `chrono` 0.4, `anyhow` 1, `thiserror` 1, `regex` 1, `once_cell` 1, `base64` 0.22, `sha2` 0.10, `semver` 1, `csv` 1, `tracing` 0.1, `tracing-subscriber` 0.3, `tracing-appender` 0.2, `sysinfo` 0.32, `libloading` 0.8, `winreg` 0.55, `discord-rich-presence` 1.1.0, `webview2-com` =0.38.2, `windows-core` =0.61.2, `windows` =0.61.3, `windows-capture` 2
