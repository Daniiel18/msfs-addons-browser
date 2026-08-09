# Contributing to SimFleet

Thanks for taking the time to help improve SimFleet — the all-in-one companion
for Microsoft Flight Simulator 2020 & 2024. This guide covers how to report
issues, request features, set up a development environment, and propose changes.

Please read the licensing note at the end before opening a pull request.

---

## Reporting bugs

Bugs are tracked on GitHub. Open a report here:

**https://github.com/Daniiel18/msfs-addons-browser/issues/new/choose**

Pick the **Bug report** template and fill it in as completely as you can. A good
report usually includes:

- **SimFleet version** (Help / About, or the title bar) and whether you're on a
  release installer or a build from source.
- **Windows version** (10 or 11, x64) and, if relevant, which simulator is
  involved (MSFS 2020, MSFS 2024, Steam or MS Store).
- **What you did, what you expected, and what actually happened** — exact steps
  to reproduce are the single most useful thing you can provide.
- **Logs.** App data lives in a portable `data/` folder next to the executable
  (falling back to `%LOCALAPPDATA%\SimFleet\data`); attach the relevant log
  file or paste the error text.
- Screenshots or a short screen capture when the issue is visual.

Before filing, a quick search of existing issues avoids duplicates. If you find
a matching report, add your details there instead of opening a new one.

Please **do not** open public issues for anything you believe is a security or
privacy problem — contact the author directly instead.

---

## Requesting features

Feature ideas are welcome. Open a new issue with the **Feature request**
template at the same link:

**https://github.com/Daniiel18/msfs-addons-browser/issues/new/choose**

Describe the problem you're trying to solve, not just the solution you have in
mind — the "why" helps shape the best fix. Note which simulator(s) and workflow
the request applies to. Check open issues first so related requests can be
consolidated.

---

## Development setup

SimFleet is a Tauri v2 desktop app: a Rust backend and a React/TypeScript
frontend running in a WebView2 shell. You'll build and run it on Windows.

### Prerequisites

- **Node.js 20+**
- **Rust 1.77+** (stable toolchain, via `rustup`)
- **Tauri v2 prerequisites** — Microsoft Edge **WebView2 Runtime** (ships with
  modern Windows) and the **MSVC build tools** (Desktop development with C++).
  See the official Tauri prerequisites guide for Windows if anything is missing.
- **Microsoft Flight Simulator 2020 and/or 2024** installed, to exercise the
  install and SimConnect features against a real setup.

### Install and run

```bash
npm install
npm run tauri:dev
```

`npm run tauri:dev` builds the Rust backend and launches the full desktop app
with hot-reloading frontend. The first Rust build is slow; later builds are
incremental.

Other useful scripts:

- `npm run dev` — frontend/demo UI only, in a plain browser. Downloads and
  installs are disabled in this mode by design (see the demo API note below).
- `npm run tauri:build` — produce release bundles. Output installer:
  `src-tauri/target/release/bundle/nsis/SimFleet_<version>_x64-setup.exe`
  (an MSI is produced as well).

---

## Code layout

A high-level map — enough to find your way around:

| Path | What lives there |
| --- | --- |
| `src/` | React + TypeScript frontend (components, screens, stores, styling). |
| `src/lib/tauri.ts` | The typed **`Api`** boundary between frontend and backend, plus the browser **demo** implementation (see below). |
| `src-tauri/src/` | Rust backend. Feature modules (`flight_log.rs`, `gsx.rs`, `install/`, `download/`, `cloud_sync.rs`, and so on). |
| `src-tauri/src/commands/` | Tauri command handlers — the functions the frontend calls through `invoke`. |
| `src-tauri/migrations/` | SQLite schema migrations (`sqlx`), numbered and applied in order. |

### Database migrations

The app uses SQLite via `sqlx` with numbered migration files in
`src-tauri/migrations/`. When you need a schema change, **add a new,
higher-numbered migration file** — never edit an existing one that has already
shipped, since users' databases have already applied it. Migrations run in
order on startup.

---

## Coding conventions

- **Match the surrounding style.** This codebase favors consistency over
  personal preference. Follow the patterns already in the file you're editing —
  naming, formatting, error handling, and module organization.
- Keep frontend and backend types in sync. The `Api` interface in
  `src/lib/tauri.ts` is the contract; backend command signatures should line up
  with it.
- Prefer small, focused changes. Unrelated cleanups belong in their own PR.
- User-facing backend messages are localized — go through the `tr!` macro and
  the dictionary in `src-tauri/src/i18n.rs` rather than hard-coding strings. The
  UI is bilingual (English/Spanish); add both when you introduce new copy.

### Keep the demo API in sync when adding a command

`src/lib/tauri.ts` defines a single `Api` interface with **two**
implementations:

- `realApi` — the real desktop backend, calling Rust through `invoke`.
- `demoApi` — a browser-only stub used by the public demo build, which has no
  Rust backend and no filesystem access.

`export const api = isTauri ? realApi : demoApi;` selects between them.

Because both objects are typed as `Api`, **adding a method to the interface
without adding a `demoApi` stub breaks the demo build's TypeScript compile.** So
whenever you add or change a command:

1. Add/adjust the method on the `Api` interface.
2. Implement it in `realApi` (wired to the new Tauri command).
3. Add a matching stub in `demoApi` — return plausible sample data or a safe
   no-op. Downloads and installs stay disabled in demo mode.

Build the frontend (`npm run build`) before opening your PR to confirm the demo
build still type-checks.

---

## Proposing changes

1. **Fork** the repository and create a topic branch off `dev`
   (`fix/gsx-icao-lookup`, `feat/logbook-filter`, etc.).
2. Make your change, keeping commits focused and messages clear.
3. Confirm it builds: `npm run tauri:dev` for the app, and `npm run build` to
   verify the demo/frontend type-checks.
4. **Open a pull request against the `dev` branch** (not `main`).
5. **Reference the issue** your PR addresses (e.g. "Fixes #123"). If there's no
   issue yet, open one first so the change can be discussed.

Describe what changed and why, and include screenshots for UI changes. Smaller,
well-scoped PRs are reviewed and merged faster.

---

## Licensing of contributions

SimFleet is **proprietary / all rights reserved**. The source is published for
transparency and learning, but it is **not** licensed for reuse,
redistribution, or derivative works.

Contributions are genuinely welcome — but by submitting a contribution (a pull
request, patch, or any other material) you agree that the author,
**Daniel Espinal (@Daniiel18)**, may use, modify, and distribute it as part of
the project under the project's license, without restriction or obligation to
you.

SimFleet is an independent, unofficial fan project. It is **not** affiliated
with, endorsed by, or associated with Microsoft, Asobo Studio, Xbox Game
Studios, FSDreamTeam/GSX, flightsim.to, SceneryAddons, SimPlaza, or any addon
developer. All trademarks belong to their respective owners.

Thanks for contributing.
