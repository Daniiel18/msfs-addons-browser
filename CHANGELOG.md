# Changelog

All notable changes to SimFleet are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [7.4.0] - Unreleased

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
