# Changelog

All notable changes to SimFleet are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [8.3.0]

### Added
- **AeroHub (aerohub.st) as a source** in Explore and search: MSFS add-ons with their VaultDrop/ModsFire links, multi-part downloads merged into one, and the install notes on the card.
- **Installers, apps and navdata** (anything that doesn't go to Community) are extracted into a folder you choose and opened in Explorer. Mixed archives install the Community packages and offer the rest. Works for every source and for drag & drop.
- **The Downloads panel follows every hoster download:** waiting, progress, processing, ready, installed or saved, failed and cancelled. Windows tells you when a download is ready, an "about to start" hint appears while the server prepares the file, and interrupted downloads resume on their own.
- **Crew VS:** finished duels show the max altitude, max ground speed and distance of both pilots.
- **Aircraft location and ferry flights:** each aircraft is where it last landed (shown in the Hangar). When your OFP departs from another airport, "Ready to fly?" offers a ferry that takes the real flight time and is charged to the airline in Finances (cancellable before it arrives). Flying anyway charges the ferry when the flight closes, only for flights from this version on.
- **Real airline operations (OpenSky):** after entering the activation code in Settings › Real flights, the airlines in Finances operate on their own with the real flights OpenSky saw (complete UTC days), priced like your flights, plus fleet fixed costs. A "Real operations" card and one balance-curve point per day show it. Nothing goes to the FlightBook.
- **Real fleets:** Finances › airline › new Fleet tab with the aircraft seen flying for that airline in the last 30 days plus yours (registration, type, flights, last landing).
- **Your aircraft fly on their own:** aircraft from your FlightBook, Fenix/PMDG TechLog and installed liveries move with their real flights (their maintenance is never touched). "Ready to fly?" tells you when the real aircraft is flying right now and lets you reserve it so it stays with you after it lands.

### Changed
- flightsim.to downloads go to the Downloads panel without blocking the page, and the flightsim.to toolbar is gone (reload button in the Explore header).
- In-app notifications use a single style; the install results panel and the "Installed" screen became regular notifications.
- Cancelling or failing the install of an internal download deletes its temporary files right away.

### Fixed
- GSX receipts are read again (GSX now bills in local currency with a USD equivalent); past flights are recalculated.
- With minimize to tray on, closing a download window no longer leaves it running hidden.
- Windows Hello unlock failing while flightsim.to was open, and sidebar tooltips hidden behind flightsim.to.
- Accented and special characters (e.g. "°") in Fenix/PMDG TechLog catalogs. Checked against Fenix TechLog 2.0.1.

## [8.2.0]

### Added
- **New FlightBook evaluation rules:** stabilized approach from 1,000 ft, stable speed below 500 ft, no bounce on touchdown, touchdown under 1.6 G, no exceedances recorded by the Tech Log and dispatched without NO-GO items.
- **The evaluation explains itself:** each missed item shows what was measured (e.g. "max IAS 273 kt") and each skipped one says why (e.g. "this aircraft doesn't pass its transponder code to the sim").
- **Tech Log note in the evaluation:** the flight's Fenix/PMDG TechLog or SimFleet Tech Log score and events appear at the top of the evaluation.

### Changed
- Your past flights are re-evaluated once with the new rules after updating.

### Fixed
- "Parking brake applied" failing on every FlyByWire and FSLabs flight (they don't pass the parking brake to the sim). New FlyByWire flights read the aircraft's own parking brake lever.
- "Parking brake applied" failing when the brake was released at the gate for the GSX pushback.
- "≤ 250 kt below 10,000 ft" and "Landing lights used" now use altitude above sea level, so approaches into high airports (Bogotá, Lhasa…) are no longer penalised above 10,000 ft.
- "Landing gear deployed" is checked from 1,000 ft above the runway instead of 1,800 ft.
- "Landing lights turned off" gives a minute to vacate the runway; taxi speed allows 2 kt of margin; the transponder rule no longer fails on the iFly 737, which doesn't publish its code.
- G-force spikes from the sim (e.g. 9 G when loading) no longer fail the G-force rules.

## [8.1.1]

### Fixed
- Clicking a maintenance alert (or "View maintenance" in "Ready to fly?") for an aircraft that is in Fenix or PMDG TechLog but was never flown with SimFleet now opens that aircraft's maintenance. These aircraft now appear in the Hangar fleet, marked "TechLog only", instead of the Hangar staying on the fleet overview.
- After updating, the taskbar and tray now show the new SimFleet logo: the installer refreshes Windows' icon cache, which kept showing the old icon.

## [8.1.0]

### Added
- **Fenix TechLog and PMDG TechLog in SimFleet.** If you use them, the Hangar shows their maintenance (GO / MEL / NO-GO, open and deferred defects, worn parts, consumables and checks), FlightBook shows each flight's TechLog score and events, and what TechLog does is charged in Finance at the airline's maintenance level. TechLog stays in charge: SimFleet reads it and sends it your orders.
- **Orders from SimFleet, even with TechLog closed.** Fix, defer, replace, service or do a check from the Hangar: if TechLog is closed, SimFleet opens it (minimised) and picks the aircraft. If the sim has another aircraft loaded, the order waits in a queue and is applied by itself when you load that one.
- **SimFleet Tech Log for every other aircraft** (iFly 737, PMDG 777, iniBuilds, FlyByWire and the rest): parts that wear with use, defects and the MEL, checks, and landings and exceedances recorded in flight. Record-only: nothing is ever set in the aircraft.
- **"Ready to fly?" checks maintenance.** A NO-GO aircraft gets a warning with a shortcut to its maintenance; departing anyway marks the flight with a dispatch violation.
- **Airline night maintenance.** With the Premium maintenance level, the airline does the pending work of SimFleet Tech Log aircraft every night.
- **Wear rate setting** (×1 realistic to ×5 demanding) in Settings › Flight & sim › Maintenance.
- **GSX gates and parking on the Scenery map.** Zoom in on an airport with a GSX profile to see its stands as pins, and pick one in the airport card.
- **flightsim.to inside Explore**, next to SceneryAddons and Simplaza.
- **New SimFleet logo** for the window, taskbar, tray and installer.

### Changed
- The Hangar's overall condition, Performance, History, Documents, the aircraft report and Finance maintenance now come from each aircraft's Tech Log. The old component wear system and its estimated reports are gone.
- Maintenance alerts in the notification bell come from the Tech Log and open the aircraft's maintenance.
- MSFS 2020 with Fenix TechLog 1.3.0: brake temperatures are worked out from the braking when the sim gives none, and a button opens TechLog's page (MSFS 2020 has no EFB).

### Fixed
- Clicking a recent flight on Home now focuses that flight in FlightBook.

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
