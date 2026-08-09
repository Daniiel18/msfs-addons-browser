# SimFleet

**All-in-one companion for Microsoft Flight Simulator 2020 & 2024 — browse, download and install addons straight into your Community folder, then track your flying with a logbook, achievements, an airline economy and more.**

[![Version](https://img.shields.io/github/v/release/Daniiel18/msfs-addons-browser?label=version&color=2563eb)](https://github.com/Daniiel18/msfs-addons-browser/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/Daniiel18/msfs-addons-browser/total?color=2563eb)](https://github.com/Daniiel18/msfs-addons-browser/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078D6?logo=windows&logoColor=white)](#requirements)
[![MSFS](https://img.shields.io/badge/MSFS-2020%20%7C%202024-1e88e5)](#requirements)
[![License](https://img.shields.io/badge/license-Proprietary-8b5cf6)](#license--disclaimer)
[![Live Demo](https://img.shields.io/badge/%F0%9F%8C%90-Live%20Demo-22c55e)](https://daniiel18.github.io/msfs-addons-browser/)

SimFleet is a Windows desktop companion for Microsoft Flight Simulator 2020 and 2024. It combines a full addon browser — with in-app downloads and one-click installation into the right Community folder — with a suite of pilot tools: a real-time logbook, per-aircraft and fleet analytics, an airline economy derived from your flights, achievements, and integrations for GSX Pro, SimBrief, Discord and more. Everything lives in a single native app, so you can go from finding a scenery to flying to it without leaving SimFleet.

<p align="center">
  <a href="https://github.com/Daniiel18/msfs-addons-browser/releases/latest"><b>⬇️ Download</b></a> ·
  <a href="https://daniiel18.github.io/msfs-addons-browser/"><b>🌐 Live Demo</b></a> ·
  <a href="https://github.com/Daniiel18/msfs-addons-browser/issues/new/choose"><b>🐛 Report a bug</b></a>
</p>

> The **Live Demo** is a public, UI-only build of the interface. Downloading and installing addons are disabled by design — it's there to explore the look and feel, not to install anything.

---

## Table of contents

- [See it in action](#see-it-in-action)
- [Features](#features)
- [Requirements](#requirements)
- [Installation](#installation)
- [Try the demo](#try-the-demo)
- [Building from source](#building-from-source)
- [How it works](#how-it-works)
- [Reporting bugs & requesting features](#reporting-bugs--requesting-features)
- [Contributing](#contributing)
- [License & disclaimer](#license--disclaimer)

---

## See it in action

🌐 **[Open the live demo →](https://daniiel18.github.io/msfs-addons-browser/)** — the real interface running in your browser with sample data.

Click through the dashboard and Addon Health Center, the addon browser, FlightBook and the Hangar. Downloading and installing are disabled in the demo; everything that touches your PC only runs in the desktop app.

<!-- Static screenshots will be added under docs/screenshots/ and embedded here. -->

---

## Features

### Addons — browse, download, install

- **Addon browser.** Search and browse MSFS addons across built-in sources (SceneryAddons, SimPlaza, Skybound) with thumbnails, developer, version, ICAO and release date.
- **In-app downloads.** An embedded BitTorrent client (librqbit) with progress, pause/resume and persistence, plus auto-install when a download completes. Web hosters (modsfire, mixdrop and others) are handled through an embedded browser with uBlock Origin Lite ad-blocking, a cancel button, multi-part file chaining, auto-password from the addon page, and automatic torrent→mirror recovery when a torrent has no seeds.
- **Installer.** Extracts RAR (including multi-volume), 7z and ZIP (including AES-encrypted) straight into the MSFS Community folder, with MSFS 2020/2024 compatibility detection so it targets the right simulator. Supports drag-and-drop install of local archives and runs a pre-flight disk-space check.
- **Installed-addons manager.** Scan the Community folder, classify packages (airport / aircraft / livery / instrument / sound / misc), enable or disable them, uninstall, and check for updates.

### Liveries & Health

- **Liveries.** A PMDG/iFly livery installer, plus a per-aircraft **Get Liveries** button that opens the matching liveries on flightsim.to — it auto-detects the model, even for encrypted study-level aircraft like PMDG and Fenix. Downloaded liveries are tracked for updates.
- **Addon Health Center.** A dashboard section with sim-compatibility checks (it flags 2024-built addons sitting in a 2020 Community folder), orphaned-livery detection and leftover-artifact cleanup, all with one-click fix / move / remove actions.

### Flying — FlightBook, Hangar, Economy, Achievements

- **FlightBook / logbook.** Real-time SimConnect flight tracking, logbook import (MSFS logbook + VAS-ACARS), flight tracks, METAR capture, damage analysis, landing scoring, and a Landing OSD overlay.
- **Best Landings recorder.** Records your landings using Windows Graphics Capture with hardware H.264 encoding and system-audio capture.
- **Hangar.** Per-aircraft and fleet analytics, maintenance tracking and alerts, and your Best Landings.
- **Airline economy.** A profit-and-loss ledger derived from your flights, reading real GSX fuel, catering and handling receipts.
- **Achievements.** 23 tiered medal badges (bronze → diamond) earned from your flight history.

### Integrations — GSX, SimBrief, Discord, Cloud, Maps

- **GSX Pro profile management.** Install profiles, group them by variant, look them up per airport by ICAO, and check for updates.
- **SimBrief integration.** Store your pilot ID, fetch OFP/briefing, list flights, link an OFP to a logbook entry, and pull route fixes.
- **Discord Rich Presence.** Shows your current flight (optional, off by default).
- **AIRAC cycle tracking** with update checks, and **flightsim.to integration** with update alerts for your tracked files.
- **Cloud sync (optional).** Sync your flight log and preferences to a private Google Drive `appDataFolder` using your own OAuth credentials, plus a folder-sync option.
- **Maps.** A MapLibre GL world map of your installed airport and scenery addons, and a routes map of the flights you've flown.

### Quality of life

- **Native Windows desktop notifications (NEW in 7.4.0).** When a download finishes or fails while SimFleet is minimized to the tray, an alert lands in the Windows Action Center — so you can keep flying and still know when your addon is ready.
- **Backup & export.** Export your Community inventory to ZIP, CSV, TXT or JSON, and import an inventory back.
- **Bilingual UI** (English / Spanish), an onboarding tour, a per-version What's New, and a command palette.
- **Desktop niceties.** Optional autostart with Windows, single-instance, and minimize-to-tray.

---

## Requirements

- **Windows 10 or 11 (x64).** Core features — SimConnect flight tracking, the Best Landings recorder, GSX support and registry access — are Windows-only.
- **Microsoft Edge WebView2 Runtime.** Ships with modern Windows; the Tauri shell and the embedded download browser both need it.
- **Microsoft Flight Simulator 2020 and/or 2024** installed, to install addons and for SimConnect flight tracking.
- **Free disk space** for downloading and extracting large scenery and aircraft archives.
- **Optional:** GSX Pro (FSDreamTeam) for the GSX features; your own Google OAuth desktop credentials for Drive cloud sync.

---

## Installation

1. Go to the [latest release](https://github.com/Daniiel18/msfs-addons-browser/releases/latest).
2. Download **`SimFleet_<version>_x64-setup.exe`** (an MSI package is also produced if you prefer it).
3. Run the installer, then launch SimFleet.

If Windows prompts about **WebView2**, install the Microsoft Edge WebView2 Runtime — it ships with modern Windows, so most systems already have it.

**Where your data lives.** App data is stored portably in a `data/` folder next to the executable. If that location isn't writable, SimFleet falls back to `%LOCALAPPDATA%\SimFleet\data`.

---

## Try the demo

Prefer to look before you install? Open the **[Live Demo](https://daniiel18.github.io/msfs-addons-browser/)** — a public browser build of the UI.

It renders the full interface so you can explore the layout and screens, but **downloads and installs are disabled by design**. Anything that touches your PC — installing into a Community folder, SimConnect tracking, GSX, the recorder — only runs in the desktop app.

---

## Building from source

**Prerequisites**

- Node.js 20+
- Rust 1.77+ (stable)
- The Tauri v2 prerequisites: Microsoft Edge WebView2 + the MSVC build tools

**Build the desktop app**

```bash
npm install
npm run tauri:build
```

The installer is written to:

```
src-tauri/target/release/bundle/nsis/SimFleet_<version>_x64-setup.exe
```

**Development**

```bash
npm run tauri:dev   # run the full desktop app in dev mode
npm run dev         # run only the web / demo UI
```

---

## How it works

SimFleet runs as a **Tauri v2** desktop shell hosting a **React + TypeScript** frontend (Vite, Tailwind CSS, Zustand, Framer Motion, Recharts, MapLibre GL) inside a WebView2 window. The UI talks to a **Rust** backend through Tauri commands, which handle everything that touches the system: scanning the Community folder, extracting archives, SimConnect flight tracking, GSX and registry access, and the recorder. Addon metadata and app state are cached in a local **SQLite** database (via `sqlx`, with migrations). Downloads run through an embedded **librqbit** BitTorrent client, while web hosters are driven through an **embedded browser** with ad-blocking, so multi-part and password-protected downloads complete automatically and hand off to the installer.

---

## Reporting bugs & requesting features

Found a bug or have an idea? Open an issue using the templates at
**[github.com/Daniiel18/msfs-addons-browser/issues/new/choose](https://github.com/Daniiel18/msfs-addons-browser/issues/new/choose)**.

Helpful details for bug reports: your Windows version, whether you're on MSFS 2020 or 2024, the SimFleet version (see **What's New** or the About screen), and steps to reproduce.

---

## Contributing

Interested in helping out? Please read [CONTRIBUTING.md](CONTRIBUTING.md) first — it covers how to file good issues, the scope of contributions the project accepts, and what to expect given the license below.

---

## License & disclaimer

**Proprietary — All Rights Reserved.** © Daniel Espinal ([@Daniiel18](https://github.com/Daniiel18)).

The source is published publicly for transparency and learning. It is **not** licensed for reuse, redistribution, or derivative works. Please do not copy, redistribute, or build derivative products from this code.

**Unofficial / not affiliated.** SimFleet is an independent, unofficial fan project. It is **not** affiliated with, endorsed by, or associated with Microsoft, Asobo Studio, Xbox Game Studios, FSDreamTeam / GSX, flightsim.to, SceneryAddons, SimPlaza, or any addon developer. All trademarks belong to their respective owners.

You are responsible for complying with the terms of service of any third-party sites the app accesses and with the licenses of any addons you download or install.
