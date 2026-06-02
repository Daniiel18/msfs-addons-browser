# Claude/Codex Brief

Token budget matters in this repo.

- Do not read the whole repository.
- Start from this file and the user's current task.
- Open `README.md` only for architecture, onboarding, or documentation tasks.
- Use `rg` to locate symbols, commands, events, stores, and components.
- Do not open `node_modules`, `dist`, `target`, `package-lock.json`, or
  `src-tauri/Cargo.lock` unless the user explicitly asks or the task requires it.
- Before editing, state the exact files you plan to change.
- Read only files directly related to the current feature or bug.
- Prefer tracing one path: UI action -> store/API wrapper -> Tauri command ->
  Rust domain module -> DB.
- For reviews, review the diff first. Do not rescan unrelated code.
- For frontend tasks, inspect the existing component/style pattern before adding
  new UI.
- For backend tasks, inspect command facade plus the domain module before
  changing logic.
- Do not paste whole files back to the user.
- Summarize changes and tests run at the end.

Common entry points:

- App shell: `src/App.tsx`
- Tauri API wrapper: `src/lib/tauri.ts`
- Backend bootstrap: `src-tauri/src/lib.rs`
- Commands: `src-tauri/src/commands/`
- Sources: `src-tauri/src/sources/`
- Community scan: `src-tauri/src/community_scanner.rs`
- Installer: `src-tauri/src/install/mod.rs`
- Downloads: `src-tauri/src/download/manager.rs`
- FlightBook: `src-tauri/src/flight_log.rs`
- SimConnect: `src-tauri/src/simconnect_watcher.rs`
- Scoring/Flight Evaluation: `src-tauri/src/scoring/{mod,rubric}.rs`
- Cloud sync: `src-tauri/src/cloud_sync.rs`
- i18n: `src/lib/i18n.ts` + `src/lib/i18n/{es,en}.json` (no hardcoded UI strings)

Build & release (only when the user asks to ship):

- Work on `dev`; merge to `main` only with explicit approval.
- Bump the SAME version in three files: `package.json`,
  `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`.
- Verify frontend with `npm run build`; full installer with `npm run tauri:build`.
- Migrations: numbered `.sql` in `src-tauri/migrations/` (sqlx auto-discovers).
  Never edit an applied migration — add the next number.
- Commit messages end with:
  `Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>`.
- Never commit `secrets.local.toml`. The user tests each build in MSFS.
