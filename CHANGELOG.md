# Changelog

All notable changes to SimFleet are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [8.0.0]

### Added
- **Brand-new interface.** Compact collapsible sidebar, a single top bar with Ctrl+K search, a modern dark theme with a teal accent (light theme kept and polished) and the bundled Inter font. Every screen and ~40 modals and panels were redesigned on the same design tokens.
- **New Home.** Welcome hero with your current, next or last flight — now with a photo of the aircraft in your SimBrief plan behind it — plus quick actions, KPIs, recent flights and a notices panel.
- **Startup lock.** SimFleet asks you to confirm it's you before opening: Windows Hello (PIN, fingerprint or face) or, as a fallback, your Windows account password. SimFleet never sees or stores your password.
- **Install queue.** A new drop or download no longer replaces the install dialog that is already open — it waits its turn. Clicking outside the dialog, pressing Escape or choosing "Later" keeps the install as pending in Notifications, with **Install** (no dialog) and **Cancel** buttons; click the card to reopen the dialog.
- **"Not mine" button** for SimBrief plans on Home and on the live map, for shared SimBrief accounts.
- **Edit flight number and callsign** of a FlightBook flight (the callsign also updates the flight's airline).
- New splash screen in the new style.
- Explore: card grid with a Download menu. Installed: new List view. FlightBook: flight details dock as a right column and year tabs. Hangar: aircraft KPI strip. Finance: global summary and cost breakdown.

### Changed
- Settings now opens as its own screen inside the app instead of a modal.
- The "Hide" list buttons were removed from FlightBook and Scenery map.
- SimBrief plans on a shared account are told apart reliably: the owner rule now applies across the whole app (with your Drive email = your plans; without it = the plans marked 357), and a plan must depart from the airport you're at, or where your open flight started, before it's used for live tracking.
- AIRAC now reads the real installed cycle from the Navigraph manifests and re-checks at local midnight.
- Simplaza is available again in MSFS 2024 mode; Skybound is hidden while the site is down.

### Fixed
- Someone else's SimBrief plan from a shared account no longer shows up as your live route in FlightBook (the latest plan of the account was being used even when it wasn't yours).
- Google Drive sync: "UNIQUE constraint" failure when downloading the snapshot, and sign-in with the renewed OAuth client.
- Map tiles showing "API key required".
- Mixdrop / Cloudflare Turnstile "Verification failed" on download pages.
- Share card rounded corners and text layout; FlightBook night overlay kept in sync with the world clock.
- Light theme: hover tints and borders no longer stick.

## [7.4.1]

### Fixed
- **Update loop.** The "new version available" prompt kept reappearing on every launch even after installing. Two root causes: (1) the crate version in `Cargo.toml` was not bumped in 7.4.0, so the app reported itself as 7.3.0 and never matched the published release; (2) locally-built binaries (run from `target/`) were offered a GitHub update the installer cannot apply. Now all version files are kept in sync, local/development builds no longer check for updates, and the startup "Skip" persists per version.

## [7.4.0]

### Added
- Native Windows desktop notifications: when a download finishes or fails while SimFleet is minimized to the tray, an alert surfaces in the Windows Action Center — so you can keep flying and still know when your addon is ready.
- Addon Health Center on the dashboard: sim-compatibility checks (flags 2024-built addons sitting in a 2020 Community folder), orphaned-livery detection, and leftover-artifact cleanup, each with one-click fix/move/remove actions. This release also ships the previously local-only 7.3.0 Health Center work.
- Public browser demo of the UI (downloads and installs disabled by design).

### Changed
- Overhauled the README and repository documentation.

## [7.2.0]

### Added
- Per-aircraft "Get Liveries" button that opens matching liveries on flightsim.to, backed by a verified slug mapping (`liveryslugs.ts`).
- Livery download tracking for update checks.
- MSFS 2020/2024 compatibility handling on install so archives target the correct simulator.
- Per-simulator scenery re-optimize.

### Changed
- Aircraft classification now keys off flight-model presence, correctly detecting encrypted study-level aircraft (PMDG, Fenix).
- Improved scenery ICAO extraction, including GSX profile lookups.

### Fixed
- AIRAC cycle now uses the local date.

## [7.1.0]

### Added
- Automatic torrent-to-mirror download recovery when a torrent has no seeds.
- Automatic cleanup of stale build artifacts.

### Changed
- Large internationalization (i18n) cleanup across the UI.

## [7.0.0]

### Added
- Achievements system with tiered medal badges derived from your flight history.
- Discord Rich Presence (optional, off by default).
- flightsim.to update center in the Addons section.

## Earlier

For 6.x and older history, see the [GitHub Releases](https://github.com/Daniiel18/msfs-addons-browser/releases).
