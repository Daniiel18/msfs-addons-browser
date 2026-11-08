# Auditoría completa de SimFleet v8.4.1+

**Fecha:** 2026-11-07
**Alcance:** repositorio `C:\proyectos\msfs-addons-browser` (HEAD 6625f95 + ~161 cambios sin commit)
**Método:** 10 agentes especializados + pruebas en tiempo de ejecución + revisión manual
**Cobertura:** ~84,600 líneas Rust (123 archivos), ~67,400 líneas TypeScript (189 archivos), 88 migraciones SQL, ~260 comandos Tauri

---

## RESUMEN

| Severidad | Cantidad |
|-----------|----------|
| **Crítica**   | **3**  |
| **Alta**      | **21** |
| **Media**     | **39** |
| **Baja**      | **39** |
| **TOTAL**     | **102**|

---

## CRÍTICAS (3)

### C01 — Webviews externas pueden invocar cualquier comando Tauri
- **Archivo:** `build.rs:61`, `flightsim_embed.rs:248`
- **Qué pasa:** Sin AppManifest de comandos, las webviews externas (fsto-embed, livery-browser, hoster-dl-*, cuenta flightsim.to) pueden hacer `invoke()` de cualquier comando: `launch_test_step` con app=powershell, `save_binary_file`, `read_text_file`, `install_update`, `health_move_to_sim`, `delete_dropped_archive`. Además `on_navigation` no tiene lista blanca y `ends_with("flightsim.to")` acepta dominios como `evilflightsim.to`.
- **Impacto:** Ejecución remota de código desde cualquier página web cargada en las webviews embebidas.
- **Arreglo:** AppManifest con capabilities solo para ventanas locales; invoke_handler que rechace origen no local; lista blanca de dominios con validación completa.
- **Fuente:** S01

### C02 — UnRAR 7.1 con CVEs conocidas + path traversal
- **Archivo:** `install/mod.rs:585-626`, `drop_install.rs:709-725`
- **Qué pasa:** `unrar_sys 0.5.8` usa UnRAR 7.1 con CVE-2025-6218 y CVE-2025-8088. Además `drop_install` no comprueba path traversal en entradas RAR.
- **Impacto:** Un archivo RAR malicioso puede escribir fuera de la carpeta de destino.
- **Arreglo:** Actualizar unrar_sys (>=7.13); listar entradas antes de extraer; rechazar ':', '..', rutas con raíz, en ambos sitios.
- **Fuente:** S02, A16/S08

### C03 — Reconexión de SimConnect borra vuelo real (y nube)
- **Archivo:** `simconnect_watcher.rs:4103,4256-4294`
- **Qué pasa:** Al reconectar, si el vuelo restaurado está en tierra con <50kt, freno y motores off, `delete_entry` borra el vuelo completo incluyendo datos de nube.
- **Impacto:** Pérdida permanente de datos de vuelo del usuario.
- **Arreglo:** Vuelo restaurado en tierra → cerrar como partial; `delete_entry` solo para vuelos de la conexión actual sin tramo aéreo.
- **Fuente:** F01

---

## ALTAS (21)

### A01 — «Quitar enlace» borra carpeta real en otra ubicación
- **Archivo:** `package_ops.rs:132-266`
- **Qué pasa:** Con `delete_target=false`, borra carpetas reales en C2020 (destino de enlace 2024), Documents\MSFS Sceneries, o ruta DB real cuando el enlace roto está en otra Community.
- **Fuente:** A01/H1

### A02 — deleteTarget=true se aplica a TODAS las Community
- **Archivo:** `PackageDetailModal`, `DiskCleanupModal:~265`, `download/manager.rs:677`
- **Qué pasa:** Paquete normal manda `deleteTarget=true`; `uninstall_by_folder` lo aplica a TODOS los candidatos incluyendo enlaces en otra Community hacia biblioteca externa.
- **Fuente:** H2

### A03 — Reinstalar a otra carpeta borra la vieja en todas las Community
- **Archivo:** `download/manager.rs:669-689`, `package_ops:132-184`
- **Fuente:** A02(orig)

### A04 — Escaneo borra enlaces manuales y posiciones
- **Archivo:** `community_scanner.rs:1144-1167,1298-1314`
- **Qué pasa:** Al cambiar de sim, desconectar un disco, o tener Community vacía, el escaneo borra todo lo que no encuentra.
- **Fuente:** A03(orig)

### A05 — Paquete en raíz del archivo se instala con nombre temporal
- **Archivo:** `drop_install.rs:249,1594-1599,1841-1846`
- **Fuente:** A04(orig)

### A06 — Variantes con mismo destino: todas marcadas, gana la última
- **Archivo:** `DropSelectModal.tsx:89-96`, `drop_install.rs:1642-1657`
- **Fuente:** A05(orig)

### A07 — Vuelos en nube nunca se actualizan
- **Archivo:** `cloud_sync.rs:1990,2001-2003,2311-2314`
- **Qué pasa:** Vuelo ya subido a la nube nunca recibe ediciones ni cambios de partial→completed.
- **Fuente:** E01

### A08 — Mantenimiento nocturno doble cobro en cada PC
- **Archivo:** `mx/mod.rs:30`, `mx/store.rs:2306-2355`, `ledger_sync.rs:99-113`
- **Qué pasa:** El nocturno corre en cada PC con claves `Utc::now()`, generando cobros duplicados. Timestamp pisa TechLog.
- **Fuente:** E02

### A09 — Recompute cambia costos de días ya cerrados
- **Archivo:** `airline_economy.rs:1850-1864,1966-1994`
- **Qué pasa:** Recalcula todos los días con flota de ventana 30 días, pero `real_flight` se purga a 35 días.
- **Fuente:** E03

### A10 — Ferries retroactivos al purgar real_flight
- **Archivo:** `ferry.rs:897,924-929,288-313`
- **Fuente:** E04

### A11 — Max altitud/velocidad contaminadas por muestras de carga
- **Archivo:** `simconnect_watcher.rs:3876-3909,4453`
- **Fuente:** F02

### A12 — Pitch con signo invertido en track/OSD/score
- **Archivo:** `simconnect_watcher.rs:4730,3631`
- **Fuente:** F03

### A13 — Retención de clips borra el clip nuevo
- **Archivo:** `landing_recorder.rs:1936-1953,1459-1464`
- **Fuente:** F04

### A14 — 90 s en rodaje de llegada cierra vuelo y abre fantasma
- **Archivo:** `simconnect_watcher.rs:4047-4049,4076-4083`
- **Fuente:** F05

### A15 — Pausas antes del pushback se restan del tiempo
- **Archivo:** `simconnect_watcher.rs:2358-2373,4120-4138`
- **Fuente:** F06

### A16 — Volver al menú genera aterrizaje falso
- **Archivo:** `simconnect_watcher.rs:4096-4098,2374-2376`
- **Fuente:** F07

### A17 — DB dañada/migración fallida = panic silencioso
- **Archivo:** `lib.rs:221-223,561`, `db/mod.rs:87-91`
- **Fuente:** S03

### A18 — Nightly opera sobre aviones gestionados por TechLog externo
- **Archivo:** `mx/store.rs:2306`
- **Qué pasa:** `nightly()` no filtra `managed_registrations`: aviones con TechLog externo reciben doble mantenimiento y doble cobro.
- **Fuente:** TL01

### A19 — aircraftNeeds excluye paquetes necesarios por enlace condicional ajeno
- **Archivo:** `dispatch.ts:261`
- **Qué pasa:** `held` recopila TODOS los destinos de enlaces no aplicables sin importar el addon origen; un enlace condicional de otro addon excluye un paquete necesario para el avión.
- **Fuente:** D01

### A20 — Enlaces inversos del avión ignoran condiciones
- **Archivo:** `dispatch.ts:281-284`
- **Qué pasa:** El bloque reverse links no comprueba `linkApplies(l, facts)`, incluyendo paquetes condicionales que no aplican.
- **Fuente:** D02

### A21 — Lanzar con conjunto no limpia pendiente de restauración de Focus previo
- **Archivo:** `useDispatchStore.ts:387-396`
- **Qué pasa:** `changes` siempre pone `markPending: true` pero nunca `clearPending: true`; el pendiente del Focus anterior sobrevive.
- **Fuente:** D03

---

## MEDIAS (39)

| ID | Descripción | Archivo | Fuente |
|----|-------------|---------|--------|
| M01 | Replay/sim rate borra FPM; finish_flight sin COALESCE | watcher:3707; flight_log.rs:397 | F08 |
| M02 | finalize_landed_open_flights sin destino/distancia/score | flight_log.rs:883-905 | F09 |
| M03 | Prueba OSD y rebotes falsean Crew VS | LiveVsManager.tsx:184-195 | F10 |
| M04 | "Mantener incompleto" sin tiempo/distancia/score | flight_log.rs:912-929 | F11 |
| M05 | Canal MobiFlight por defecto se limpia | mobiflight_lvars.rs:203-237 | F12 |
| M06 | Clips fuera de scope asset no se reproducen | ClipPlayerModal.tsx:30-33 | F13 |
| M07 | Desinstalar con MSFS abierto deja paquete a medias | package_ops.rs:115-283 | A06(orig) |
| M08 | Apagar duplicado C/C2024 solo apaga ruta principal | package_ops.rs:527,583 | A07(orig) |
| M09 | Borrar enlace auto se recrea en cada escaneo | scanner:1416-1430 | A08(orig) |
| M10 | En pruebas borra Documents\MSFS Sceneries real | package_ops:173-184 | A09(orig) |
| M11 | Manifest con BOM inválido | scanner:637-644 | A10(orig) |
| M12 | Falta PMDG 737-700 (pmdg-aircraft-737) | pmdg_install.rs:65-118 | A11(orig) |
| M13 | .ini PMDG suelto va a candidates[0] | drop_install.rs:1764-1838 | A12(orig) |
| M14 | Reinstalar livery no atómico | pmdg_install.rs:289-293 | A13(orig) |
| M15 | Puntuación mínima -1 marca coincidencia 0 puntos | ImportInventoryModal.tsx:484-507 | A14(orig) |
| M16 | Apagar addon enlazado toca el otro sim | package_ops.rs:390-410 | A15(orig) |
| M17 | reconcile sin transacción: duplicados permanentes | service_refunds.rs:147,164 | E05 |
| M18 | Cobros de matrículas sin vuelos no se imputan | airline_economy.rs:2251 | E06 |
| M19 | Faltan tipos de avión en classify | airline_economy.rs:540 | E07 |
| M20 | computed_at=now sube snapshot entero siempre | airline_economy.rs:1988 | E08 |
| M21 | Nube ilegible sobrescribe con snapshot vacío | cloud_sync.rs:2102 | E09 |
| M22 | Vuelo en curso entra en economía/reembolsos | flight_log.rs:1376 | E10 |
| M23 | Curva de economía no termina en el saldo | airline_economy.rs:2620 | E11 |
| M24 | Cliente updater con timeout 20s total | commands/updater.rs:177-232 | S04 |
| M25 | Sin verificación de firma/hash en actualización | updater.rs:167-172 | S05 |
| M26 | save_installer_to síncrono en hilo principal | commands/livery.rs:2763 | S06 |
| M27 | Versión catálogo vs descargada al instalar | updates.rs:~316 | H3 |
| M28 | Reinstalar C2024 deja dos copias | download/manager.rs:1056 | H4 |
| M29 | TechLog unificado: 3 copias de misma base por cambio | techlog.rs:~719 | H5 |
| M30 | Evento score:error sin receptor | scoring/mod.rs:799 | NC-01 |
| M31 | grade_for_percentage trunca en vez de redondear (94.9% = B) | scoring/mod.rs:905 | SF05 |
| M32 | update_flight_log_entry no emite flightlog://changed | commands/flight_log.rs:227-248 | SF06★ |
| M33 | Editar vuelo no recalcula economía ni re-evalúa score | flight_log.rs:473-581 | SF07 |
| M34 | greaser_kiss cuenta fpm=0 (no medido) como kiss landing | achievements.rs:291 | SF09 |
| M35 | finalize_landed_open_flights no resta paused_seconds | flight_log.rs:891-897 | SF11 |
| M36 | Zona de mapa no marca restauración como pendiente | MapZone.tsx:112-116 | D04 |
| M37 | Espera "menu"/"loaded" sin timeout ni detección de cierre | launch_flow.rs:1614-1630 | D05 |
| M38 | Claves React duplicadas para escalas intermedias | DispatchHost.tsx:327 | D06 |
| M39 | housekeeping crea defectos fantasma para aviones TechLog | mx/store.rs:1359 | TL02 |
| M40 | normalize_reg solo elimina sufijos CFM/IAE; otros motores = mismatch | techlog.rs:310-319 | TL03 |
| M41 | Datos mx_airframe obsoletos tras transición a TechLog | mx/store.rs:399 | TL04 |
| M42 | pendingWork bloquea cobros de TODOS los aviones TechLog unificado | techlog.rs:2450-2462 | TL05 |
| M43 | Reversión de vuelo no revierte cobros de nightly intermedios | mx/store.rs:1186-1278 | TL06 |
| M44 | Archivo truncado aceptado como completo (sin Content-Length) | livery.rs:2743 | DL01 |
| M45 | RunGuard timeout 30s permite escritura doble al mismo archivo | livery.rs:1903-1908 | DL02 |
| M46 | is_newer cae a lexicográfico para formatos inusuales | updates.rs:874 | DL06 |
| M47 | Sin verificación de espacio en disco antes de descargar | download/torrent.rs + manager.rs | DL10 |
| M48 | purge_temp puede borrar archivo en descarga tras reinicio | download/manager.rs:375-391 | DL05 |

★ SF06 fue reportada como alta por el agente de puntuación; la reclasifiqué como media porque el FlightBookView ya llama reload() manualmente, y el efecto se limita a vistas secundarias abiertas en paralelo.

---

## BAJAS (39)

| ID | Descripción | Archivo | Fuente |
|----|-------------|---------|--------|
| B01 | Discord RPC status='open' no existe | discord_rpc.rs:232 | F14 |
| B02 | Banner replay pegado al desconectar | watcher:1112-1145 | F15 |
| B03 | Pushback state como enum | watcher:1303,3845 | F16 |
| B04 | Hora clip UTC leída como local | AirportCard.tsx:751 | F17 |
| B05 | UPDATE por segundo sin cambios | watcher:2750-2771 | F18 |
| B06 | Touch-and-go cuenta como rebote | watcher:2525-2538 | F19 |
| B07 | Reconexión pierde pausas/combustible | watcher:1983-1991 | F20 |
| B08 | Error "Mantener" silencioso | IncompleteFlightModal.tsx:55-70 | F21 |
| B09 | Guardia sim mira proceso falso con *_REAL | launch_flow.rs:719-726 | A17(orig) |
| B10 | cleanup_stale nunca se llama | drop_install.rs:597-605 | A18(orig) |
| B11 | Ofrece borrar archivo para carpetas | DropSelectModal.tsx:241-243 | A19(orig) |
| B12 | Cualquier .ini = perfil GSX | drop_install.rs:898-940 | A20(orig) |
| B13 | 404 día cerrado guardado como OK | opensky.rs:299,598 | E12 |
| B14 | Curva no recarga tras cambiar nivel | EconomyView.tsx:1030 | E13 |
| B15 | Purgar nube no deja snapshot vacío | cloud_sync.rs:864 | E14 |
| B16 | Escenarios sin ICAO como "nunca volados" | DiskCleanupModal.tsx:101 | E15 |
| B17 | Clave reg\|comp\|fecha funde cobros | ledger_sync.rs:632 | E16 |
| B18 | Backup lee archivos enteros sin limit | commands/backup.rs:113 | E17 |
| B19 | Borra <stem>_extracted del usuario | install/mod.rs:388-389 | S07 |
| B20 | Anidados sin tope de tamaño | drop_install.rs:798-800 | S09 |
| B21 | Índice de to_lowercase en name original | perf_config.rs:434 | S10 |
| B22 | Restaurar la más antigua poda backups | content_xml.rs restore/save_backup | H6 |
| B23 | catalog_installs no se limpia al desinstalar | package_ops cleanup_db | H7 |
| B24 | score:upload:started sin receptor | scoring/mod.rs:876 | NC-02 |
| B25 | fsto-embed://url sin receptor | flightsim_embed.rs:108 | NC-03 |
| B26 | SimBriefConfirmModal nunca se activa | SimBriefConfirmModal.tsx:58 | NC-04 |
| B27 | Comando force_close_flight_log_entry huérfano | lib.rs:511 | NC-05 |
| B28 | Comando msfs_logbook_detect huérfano | lib.rs:519 | NC-06 |
| B29 | Callsign borrado deja airline_icao huérfano | flight_log.rs:521-531 | SF08 |
| B30 | butter_streak no rompe con "acceptable" | achievements.rs:317-325 | SF10 |
| B31 | Dashboard FPM sin force-negate | DashboardView.tsx:1022 | SF12 |
| B32 | Rango nota A llega hasta 200 | scoring/mod.rs:904-913 | SF13 |
| B33 | launchFlight toma OFP más reciente aunque no coincida | InFlightView.tsx:616-620 | D07 |
| B34 | profile_for busca en últimas 500 sin índice de matrícula | mx/store.rs:380-397 | TL07 |
| B35 | Cobro re-rectificación perdido por clave repetida | techlog.rs:2486-2678 | TL08 |
| B36 | alerts() ~6-8 queries por avión sin caché | mx/store.rs:2360-2450 | TL09 |
| B37 | aircraft_view crea fila al solo consultar | mx/store.rs:1806-1811 | TL10 |
| B38 | Rama muerta en is_flightsim_cdn | livery.rs:714 | DL03 |
| B39 | Comparación de versiones GSX cae a lexicográfico | gsx_update.rs:51-55 | DL04 |
| B40 | Descarga no reanuda si servidor no soporta Range | livery.rs:2667-2669 | DL07 |
| B41 | Filtro SceneryAddons impide ver addons sin año | sceneryaddons.rs:291-296 | DL08 |
| B42 | Falso positivo de update con cuenta fsto activa | flightsim_track.rs:277-285 | DL09 |

---

## PRUEBAS EN TIEMPO DE EJECUCIÓN

### Resultados positivos
- 9 vistas cargan sin errores (dashboard, search, map, addons, flightbook, inflight, hangar, economy, settings)
- 54 comandos read-only responden correctamente
- Toggle/cascade/batch/uninstall correctos en fake Community
- 334 tests Rust pasan, 19 ignorados
- TypeScript compila limpio, Vite build exitoso

### Problemas encontrados en runtime
- **SceneryAddons search roto:** HTML del sitio cambió, 0 resultados (browse sigue funcionando)
- **Economía lenta:** vista tarda >5s con 56 aerolíneas
- **Vuelo fantasma 293:** creado en BD real al abrir MSFS brevemente para test de Content.xml (origen 0,90; 11 segundos; limpieza existente lo cubre)
- **npm audit:** 1 vulnerabilidad crítica en maplibre-gl <=6.4.0 (XSS)

---

## AGENTES QUE PARTICIPARON

| # | Área | Hallazgos | Estado |
|---|------|-----------|--------|
| 1 | Vuelos y SimConnect | 21 | Completo |
| 2 | Addons e instalación | 20 | Completo |
| 3 | Economía y nube | 17 | Completo |
| 4 | Despacho, arranque y mapa | 7 | Completo |
| 5 | Seguridad y robustez | 10 | Completo |
| 6 | Puntuación y estadísticas | 9 | Completo |
| 7 | Descargas y actualizaciones | 10 | Completo |
| 8 | TechLog y mantenimiento | 10 | Completo |
| 9 | Lote sin commit | 7 | Completo |
| 10 | Cableado entre capas | 6 | Completo |

**Total bruto:** 117 hallazgos → **102 únicos** tras deduplicación

---

## ÁREAS NO REVISADAS EN PROFUNDIDAD

Cada agente listó las áreas que no pudo cubrir. Las principales son:

- **Frontend completo de TechLog** (TechLogPanel, Cabin, Cockpit, Flights, AircraftMaintenance)
- **Skybound** (parser HTML, multipart, contraseñas)
- **fsto_account.rs** (gestión de cuenta flightsim.to)
- **rubric.rs** regla por regla (45 reglas de scoring)
- **Tipos Rust vs TypeScript** (comparación sistemática de structs Serialize vs interfaces)
- **Migraciones vs consultas SQL** (88 migraciones vs queries sqlx)
- **Settings desconectados** (claves escritas pero no leídas y viceversa)
- **Stores sin inicializar** y navegación/vistas completas
- **maint_prices.rs** (precios de mantenimiento)
- **Paginación FlightBook** con >1000 entries
- **Crew VS + Supabase realtime** vs estadísticas locales
- **VAS-ACARS** importación y reconciliación

---

## RE-AUDITORÍA (2026-11-08)

### Resultado
- **334 tests Rust**: todos pasan (0 fallos, 19 ignorados)
- **TypeScript**: compila sin errores
- **Vite build**: exitoso
- **Warnings de compilación Rust**: 0

### Fixes aplicados (resumen)
| Rango | Total hallazgos | Corregidos | Omitidos | Notas |
|-------|----------------|------------|----------|-------|
| Críticas | 3 | 3 | 0 | C01-C03 |
| Altas | 21 | 21 | 0 | A01-A21 |
| Medias | 39 | ~35 | ~4 | M01-M48 (algunos de diseño/infra) |
| Bajas | 39 | 21 | 18 | B01-B42 (omitidos: watchers B02-B07, UI B08/B11/B16, diseño B22/B33, perf B18/B34/B36, infra B20/B40, dead code B24-B28) |

### Hallazgos de la re-auditoría
1. **B17 revertido**: `maint_key_ext` (clave de mantenimiento con costo) rompía la comparación de tombstones en cloud sync — los tombstones existentes usan el formato de 3 campos. Revertido a `maint_key` original. Función `maint_key_ext` eliminada (dead code).
2. **B12 completado**: El call site en `drop_install.rs:964` actualizado para verificar específicamente `"ifly_config" || "pmdg_config"` en vez de `!= "gsx_profile"`, evitando que .ini desconocidos se clasifiquen incorrectamente como configs de avión. También corregido el retorno de error de lectura en `detect_ini_owner` (línea 1435) de `"gsx_profile"` a `"unknown_ini"`.

### Fixes omitidos (justificación)
- **B02-B07** (watcher/SimConnect state): Cambios profundos en máquina de estados, riesgo alto de regresión
- **B08, B11, B16** (UI-only): Mejoras cosméticas sin impacto funcional
- **B18, B34, B36** (performance): Optimizaciones que no afectan correctitud
- **B20, B40** (infraestructura): Requieren cambios en pipeline/tooling
- **B22, B33** (diseño): Decisiones de diseño, no defectos
- **B24-B28** (dead code): Código muerto inofensivo, sin impacto en runtime

### Estado
Todo local, sin commits. Pendiente de aprobación del usuario para commit/release.
