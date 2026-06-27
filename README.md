# SimFleet

Desktop companion app for Microsoft Flight Simulator. It started as
`MSFS Addons Browser`, but the current product is broader: addon catalog,
Community manager, update checker, installer, GSX helper, SimBrief/ACARS
integration, and FlightBook telemetry.

Current app version: `6.2.7`.

## What It Does

SimFleet helps manage a local MSFS setup:

- Search public addon catalogs from SceneryAddons and Simplaza.
- Browse recent catalog entries without typing a query.
- Cache catalog metadata locally in SQLite.
- Detect the MSFS Community folder for MSFS 2020/2024, Steam/MS Store.
- Scan installed Community packages and classify airports, aircraft, liveries,
  instruments, sound/misc, and unknown packages.
- Show installed airports on a MapLibre world map.
- Detect addon updates by comparing local package versions against cached
  catalog versions.
- Install `.zip`, `.rar`, and `.7z` archives into Community.
- Reject `.ptp` PMDG liveries explicitly because the format is encrypted and
  must be installed with PMDG Operations Center.
- Manage torrent downloads with progress, pause/resume, persistence across app
  restarts, and automatic install after download.
- Open mirror/direct downloads in the system browser.
- Look up and install GSX profiles, plus detect local GSX profile ICAOs.
- Track real flights through SimConnect on Windows.
- Fall back to process detection plus SimBrief when SimConnect is unavailable.
- Store a FlightBook with routes, tracks, landing metrics, fuel, pax/cargo,
  gates, aircraft metadata, scoring, and weather samples.
- Import VAS-ACARS flights and MSFS logbook data where available.
- Sync app data through Google Drive `appDataFolder` or a user-selected folder.
- Backup and export the Community inventory as ZIP, CSV, TXT, or JSON.
- Check GitHub Releases for app updates and install Windows release assets.

## Tech Stack

- Desktop shell: Tauri v2
- Frontend: React 18, TypeScript, Vite, Tailwind, Zustand, Framer Motion
- Maps: MapLibre GL
- Charts: Recharts
- Icons: lucide-react
- Backend: Rust 2021
- Database: SQLite through `sqlx`
- HTTP and scraping: `reqwest`, `scraper`
- Torrents: `librqbit`
- Archives: `zip`, `unrar`, `sevenz-rust2`
- Flight telemetry: custom SimConnect FFI loaded from `SimConnect.dll`
- Logging: `tracing`

## Main Screens

- Dashboard: totals, disk usage, package breakdown, top creators, largest
  packages, recent installs, and update count.
- Search: SceneryAddons/Simplaza search and browse, download methods, GSX
  summary, changelog, install/download actions.
- Map: installed scenery airports, update markers, GSX/no-GSX markers, airport
  sidebar, package detail modal.
- Addons: non-airport packages grouped by type, aircraft livery scanner,
  thumbnails, updates, package detail modal.
- FlightBook: real and imported flight history, route map, live/preflight
  status, airline filters, score checklist, performance charts, weather modal.
- Hangar: per-aircraft analytics (telemetry, landing-FPM health), fleet
  overview (most-flown aircraft/airlines/destinations/regions), Best Landings.
- Finance: airline economy built from your flight history. One canonical airline
  per flight (livery names collapse to the real carrier; cargo vs passenger told
  apart by callsign/pax). Per airline: market value ("bank"), revenue (tickets or
  freight + ancillaries) vs costs (fuel, catering, handling, maintenance, taxes,
  plus real GSX `Receipts` invoices when matched), net/margin/balance, and a
  holistic passenger-satisfaction score (landings, reliability, services, load
  factor). Management ecosystem: pick a maintenance level and toggle onboard
  services (snacks, meals, wifi, seats, priority, baggage) — each with adoption,
  cost and possible loss, recomputed live. Click an airline in the Hangar
  "Most Flown Airlines" to jump straight to its Finance detail.
- Settings: theme/language, autostart, tray behavior, SimBrief, display toggles,
  folders, backup/export/import, GSX tools, cloud/folder sync, cache reset.

## How It Works

### Startup

`src/App.tsx` runs a splash bootstrap:

1. Check app version/update.
2. Load sources.
3. Load settings.
4. Bootstrap SimBrief, FlightBook, downloads, and GSX local state.
5. Scan Community.
6. Refresh update notifications.
7. Preload catalog page 1 for each source.
8. Resize the Tauri window into the normal app layout.

The backend startup lives in `src-tauri/src/lib.rs`:

- Resolve a portable data directory.
- Migrate legacy app data when possible.
- Initialize logging.
- Open SQLite and run migrations.
- Create the shared HTTP client.
- Register source scrapers.
- Restore persisted download jobs.
- Load airport data in the background.
- Spawn the SimConnect watcher.
- Start daily best-effort cloud sync if configured.

### Data Location

The app tries to store generated data beside the executable:

```text
<install-dir>/data/
  msfs-addons.db
  logs/
  temp/
  torrents/
```

If that directory is not writable, it falls back to:

```text
%LOCALAPPDATA%/SimFleet/data/
```

Older data from `%APPDATA%/org.n0xful.msfsaddonsbrowser` is migrated
best-effort on startup.

### Frontend To Backend

The frontend never calls Rust directly. It uses `src/lib/tauri.ts`, which wraps
Tauri `invoke()` calls and event listeners.

The Rust command facade is under:

```text
src-tauri/src/commands/
```

The shared backend state is `AppState` in:

```text
src-tauri/src/lib.rs
```

### Catalog Sources

Sources implement the `Source` trait in:

```text
src-tauri/src/sources/
```

Current sources:

- `sceneryaddons.rs`
- `simplaza.rs`

Both are public-site scrapers, not official APIs. They search WordPress pages,
parse result articles, fetch detail pages with bounded concurrency, and extract
download methods.

Because they depend on third-party HTML, selector changes on those sites can
break search or download extraction.

### Community Scan

The scanner reads immediate subfolders of the Community directory. A valid MSFS
package is identified by `manifest.json`. Installed package data is persisted in
the `community_packages` table.

Important behavior:

- ICAO extraction is conservative and only applied to `SCENERY`.
- `AIRCRAFT` packages with dependencies are treated as liveries.
- Invalid manifests fall back to folder-derived metadata when possible.
- Coordinates come from the local airport dataset joined by ICAO.

### Install And Download

Manual install:

1. User selects an archive.
2. Rust extracts to a temporary folder.
3. The installer searches for directories containing `manifest.json` and
   `layout.json`.
4. Each package is copied into Community.
5. The install is recorded in SQLite.
6. Temporary extraction is cleaned automatically.

Torrent download:

1. Download job is created and persisted.
2. Hoster URL is resolved to a magnet when needed.
3. `librqbit` downloads into `data/torrents/job-<uuid>`.
4. Progress is emitted through `download://update`.
5. On completion, the primary archive is installed.
6. The torrent job folder is cleaned.

Mirror/direct downloads are opened in the user's browser.

### FlightBook

The preferred tracker is SimConnect:

- Loads `SimConnect.dll` at runtime.
- Subscribes to user-aircraft simvars.
- Emits live status through `flight://current`.
- Creates a flight row at block-out/takeoff fallback.
- Inserts track samples during flight.
- Closes the flight on arrival/engine shutdown/fallback timeout.
- Emits `flightlog://changed` when the UI should refresh.

Fallback mode:

- Detects `FlightSimulator.exe` / `FlightSimulator2024.exe`.
- Uses recent SimBrief OFP data to show "flying now" context.
- Does not provide live position/telemetry if SimConnect is unavailable.

Scoring:

- Rules live in `src-tauri/src/scoring/`.
- Scores are persisted into `flight_log_score_item`.
- Summary score is stored on `flight_log`.
- Finished flights trigger scoring and best-effort cloud upload.

### Sync

Google Drive sync stores one JSON snapshot named:

```text
msfs-addons-data.json
```

It uses Google Drive `appDataFolder`, so the file is app-private and not shown
in the user's normal Drive files. OAuth uses a loopback callback.

Folder sync is a simpler alternative: write/read the same JSON snapshot from a
folder selected by the user, such as OneDrive, Dropbox, iCloud, or Google Drive
Desktop.

## Repository Layout

```text
.
|-- src/                         Frontend React app
|   |-- App.tsx                  App shell, bootstrap, routing between views
|   |-- components/              UI views, modals, panels, maps
|   |-- stores/                  Zustand stores
|   |-- lib/                     Tauri API wrapper, shared types, i18n
|   `-- styles/                  Tailwind/global CSS
|
|-- src-tauri/                   Rust backend and Tauri config
|   |-- src/
|   |   |-- lib.rs               App bootstrap, AppState, plugins, handlers
|   |   |-- commands/            Tauri command facade
|   |   |-- db/                  SQLite init and repository helpers
|   |   |-- sources/             SceneryAddons/Simplaza scrapers
|   |   |-- download/            Download manager and torrent engine
|   |   |-- install/             Archive extraction and Community install
|   |   |-- community*.rs        Community detection and package scanner
|   |   |-- simconnect*.rs       SimConnect FFI and watcher
|   |   |-- flight_log.rs        FlightBook persistence/domain logic
|   |   |-- scoring/             Virtual-airline-style scoring rubric
|   |   |-- cloud_sync.rs        Google Drive and folder sync
|   |   |-- simbrief.rs          SimBrief import/matching
|   |   |-- gsx*.rs              GSX lookup, install, parking helpers
|   |   `-- updater.rs           GitHub Releases updater
|   |-- migrations/              SQLite migrations
|   |-- resources/               Bundled airports CSV and SimConnect.dll
|   `-- tauri.conf.json          Tauri window, bundle, resources
|
|-- package.json                 Node scripts and frontend deps
|-- Cargo.toml                   Rust deps inside src-tauri/
`-- README.md
```

## Requirements

- Windows 10/11.
- WebView2 runtime.
- Node.js 20 or newer.
- Rust 1.77 or newer.
- Visual Studio C++ build tools for native Rust dependencies.
- Microsoft Flight Simulator for live tracking and Community folder detection.
- SimConnect support for full live telemetry. A `SimConnect.dll` resource is
  bundled for production builds.

## Development

Install frontend dependencies:

```bash
npm install
```

Run the Tauri app:

```bash
npm run start
```

Run only the Vite frontend demo:

```bash
npm run dev
```

Build frontend:

```bash
npm run build
```

Build desktop bundle:

```bash
npm run tauri:build
```

Run Rust tests:

```bash
cd src-tauri
cargo test
```

## Useful Entry Points

- App bootstrap and view routing: `src/App.tsx`
- Tauri API wrapper: `src/lib/tauri.ts`
- Shared frontend types: `src/lib/types.ts`
- Main backend bootstrap: `src-tauri/src/lib.rs`
- Command list: `src-tauri/src/commands/mod.rs`
- Search commands: `src-tauri/src/commands/search.rs`
- Source trait: `src-tauri/src/sources/mod.rs`
- SceneryAddons scraper: `src-tauri/src/sources/sceneryaddons.rs`
- Simplaza scraper: `src-tauri/src/sources/simplaza.rs`
- Community detection: `src-tauri/src/community.rs`
- Community scanner: `src-tauri/src/community_scanner.rs`
- Installer: `src-tauri/src/install/mod.rs`
- Download manager: `src-tauri/src/download/manager.rs`
- SimConnect watcher: `src-tauri/src/simconnect_watcher.rs`
- FlightBook DB/domain: `src-tauri/src/flight_log.rs`
- Scoring: `src-tauri/src/scoring/mod.rs`
- Cloud sync: `src-tauri/src/cloud_sync.rs`
- Updater: `src-tauri/src/updater.rs`

## Known Limits

- SceneryAddons and Simplaza are scraped from public HTML. No API contract.
- Mirror/direct downloads are opened in the browser and completed by the user.
- Torrent auto-install only works when the downloaded payload contains a
  supported archive with valid MSFS package folders.
- `.ptp` is not supported.
- Full live FlightBook data requires Windows and SimConnect.
- Some imported VAS/MSFS logbook flights have partial telemetry, so scoring and
  charts may hide unavailable series instead of inventing values.
- Update detection is heuristic for non-scenery addons because aircraft and
  misc packages usually do not have ICAO identifiers.
- The updater is custom GitHub Releases logic, not `tauri-plugin-updater`.

## Working With AI Agents Without Burning Context

This repo is large enough that "read the whole project" wastes model context.
Use small, targeted prompts and make the agent prove what it read.

### Recommended Prompt Pattern

Use prompts like this:

```text
Context budget is important. Do not read the whole repo.
Goal: <one concrete task>.
First inspect only:
- README.md
- package.json
- src/App.tsx
- src/lib/tauri.ts
- files directly related to <feature>
Use rg to find symbols. Summarize what you learned before editing.
Do not paste whole files back to me.
```

For bug fixing:

```text
Find the smallest path from UI action to Rust command for <bug>.
Use rg for the command/event/store names.
Read only the relevant files.
Give me the exact files you will edit before changing them.
```

For code review:

```text
Review only this diff. Do not rescan the repo unless a symbol is unknown.
Prioritize bugs, regressions, and missing tests.
```

### Token-Saving Rules

- Start each new Claude/Codex thread with a short goal and 5-10 relevant files,
  not the entire project.
- Prefer `rg "symbol_or_command"` over opening folders manually.
- Ask for file summaries first, then deeper reads only where needed.
- Ask the model to cite file paths and line numbers so you can verify quickly.
- Do not paste full `README`, `Cargo.lock`, `package-lock.json`, generated
  output, or giant components unless the task is specifically about them.
- Use `git diff -- <file>` or a small copied diff instead of sending whole
  files for review.
- Keep one thread per task. Long mixed threads accumulate stale context.
- When a task is done, start a fresh thread with the result summary and the
  current problem.
- Tell the model about dirty files up front if they matter.
- For frontend work, ask it to inspect the existing component and CSS pattern
  before proposing new UI.
- For backend work, ask it to trace `frontend action -> api wrapper -> Tauri
  command -> domain module -> DB`.
- For performance or crash bugs, provide logs and reproduction steps first.
- For broad planning, request a short plan only; do implementation in a fresh
  follow-up thread.

### Good Files To Give An Agent First

For app identity or architecture:

- `README.md`
- `package.json`
- `src-tauri/Cargo.toml`
- `src/App.tsx`
- `src-tauri/src/lib.rs`
- `src/lib/tauri.ts`

For search/catalog bugs:

- `src/stores/useAppStore.ts`
- `src/components/SearchBar.tsx`
- `src/components/ResultsList.tsx`
- `src/lib/tauri.ts`
- `src-tauri/src/commands/search.rs`
- `src-tauri/src/sources/sceneryaddons.rs`
- `src-tauri/src/sources/simplaza.rs`

For Community/addon management:

- `src/stores/useCommunityStore.ts`
- `src/components/MapView.tsx`
- `src/components/AddonsView.tsx`
- `src-tauri/src/commands/community.rs`
- `src-tauri/src/community.rs`
- `src-tauri/src/community_scanner.rs`
- `src-tauri/src/package_ops.rs`

For install/download:

- `src/components/DownloadsPanel.tsx`
- `src/stores/useDownloadsStore.ts`
- `src-tauri/src/commands/downloads.rs`
- `src-tauri/src/download/manager.rs`
- `src-tauri/src/download/torrent.rs`
- `src-tauri/src/install/mod.rs`

For FlightBook:

- `src/stores/useFlightLogStore.ts`
- `src/components/FlightBookView.tsx`
- `src/components/RoutesMapView.tsx`
- `src/components/PerformanceModal.tsx`
- `src/components/WeatherModal.tsx`
- `src-tauri/src/commands/flight_log.rs`
- `src-tauri/src/flight_log.rs`
- `src-tauri/src/simconnect_watcher.rs`
- `src-tauri/src/scoring/mod.rs`

For settings/sync:

- `src/components/SettingsModal.tsx`
- `src/stores/useSettingsStore.ts`
- `src-tauri/src/commands/settings.rs`
- `src-tauri/src/commands/cloud.rs`
- `src-tauri/src/cloud_sync.rs`

### Tiny Agent Brief

This repo includes a short `CLAUDE.md` for Claude/Codex-style agents. Keep it
short. If that file becomes long, it will also start wasting context.

The intended shape is:

```text
Do not read the whole repo. Start from CLAUDE.md and the current task.
Use rg to locate symbols.
Read only files related to the current task.
Do not open package-lock.json, Cargo.lock, dist, node_modules, or target unless asked.
Before edits, state the exact files to change.
After edits, summarize files changed and tests run.
```

Keep the agent brief under 40 lines.

## Legal And Safety Notes

SimFleet queries public pages from SceneryAddons, Simplaza, flightsim.to, GitHub,
Google, OpenStreetMap, OurAirports, Open-Meteo, SimBrief, and local MSFS files
depending on the feature used.

Downloads and installs are user-initiated. Mirror/direct links open in the
browser. Torrent and archive installs write to the detected or selected
Community folder. Back up your Community folder before large batch changes.
