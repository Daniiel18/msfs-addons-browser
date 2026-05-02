# MSFS Addons Browser

Aplicación de escritorio (Tauri v2 + React + Rust) para buscar, gestionar e instalar
escenarios y addons de Microsoft Flight Simulator 2020 desde fuentes públicas
(SceneryAddons, Simplaza).

## Estado actual (fase 1 — cimientos)

Implementado en esta iteración:

- Scaffold Tauri v2 + React 18 + TypeScript + Tailwind + Framer Motion
- Shell de UI con **toggle animado** entre fuentes (SceneryAddons / Simplaza)
- **Buscador** (por ICAO, nombre o desarrollador)
- **Scraper SceneryAddons** portado del proyecto .NET — filtra MSFS 2020, detecta
  desarrollador / nombre / versión / ICAO usando regex (`v\d+(\.\d+)*`, `[A-Z]{4}`)
- **SQLite** (vía `sqlx`) con schema inicial: `addons`, `installed_addons`,
  `downloads`, `logs`, `settings` + migraciones versionadas
- **Logger** estructurado (`tracing` + archivo rotado por día en `logs/app.log`)
- Estados UI: loading / empty / error / success

Pendiente para siguientes iteraciones:

- Scraper Simplaza (stub presente)
- Integración GSX (`flightsim.to/miscellaneous/gsx-pro?q=XXXX`)
- Motor de descargas (HTTP directo + torrent con `librqbit`)
- Extracción e instalación a carpeta Community de MSFS
- Vista de mapa mundial con clusters tipo `sceneryaddons.org/world-map-msfs-2020/`
- Auto-updater (Tauri updater)
- Viewer de logs en UI

## Requisitos

- **Rust** ≥ 1.77 — https://rustup.rs (descargar `rustup-init.exe` y ejecutar)
- **Node.js** ≥ 20 — ya instalado en tu sistema
- **Windows**: WebView2 (ya viene con Windows 11)
- Build tools de Visual Studio (C++ build tools) — los instala `rustup-init`
  si eliges la opción por defecto

## Primer arranque

```bash
cd app
npm install
npm run tauri dev
```

El primer `cargo build` tardará varios minutos compilando dependencias
(`tauri`, `sqlx`, `scraper`, `reqwest` + tls). Tras eso los arranques son rápidos.

## Estructura

```
app/
├── index.html
├── package.json
├── vite.config.ts
├── tsconfig.json
├── tailwind.config.js
├── src/                      # Frontend React
│   ├── App.tsx
│   ├── main.tsx
│   ├── components/
│   │   ├── SourceToggle.tsx  # Toggle animado (Framer Motion)
│   │   ├── SearchBar.tsx
│   │   ├── ResultCard.tsx
│   │   └── ResultsList.tsx
│   ├── stores/
│   │   └── useAppStore.ts    # Zustand
│   ├── lib/
│   │   ├── tauri.ts          # Wrapper de invoke()
│   │   └── types.ts          # Tipos compartidos con Rust
│   └── styles/globals.css
│
└── src-tauri/                # Backend Rust
    ├── Cargo.toml
    ├── tauri.conf.json
    ├── migrations/001_initial.sql
    ├── capabilities/default.json
    └── src/
        ├── main.rs
        ├── lib.rs            # Bootstrap + AppState
        ├── logger.rs         # tracing + rolling file
        ├── db/mod.rs         # SQLite (sqlx) + repo
        ├── parser/mod.rs     # Developer / name / version / ICAO parser
        ├── sources/
        │   ├── mod.rs        # trait Source + tipos
        │   ├── sceneryaddons.rs
        │   └── simplaza.rs   # stub
        └── commands/
            ├── search.rs     # #[tauri::command] search, list_sources
            └── addons.rs     # list_installed
```

## Arquitectura

Separación en capas:

- **UI** (`src/`): React + Zustand. Sólo conoce los tipos en `lib/types.ts` y
  llama al backend por `api` (`lib/tauri.ts`).
- **Comandos** (`src-tauri/src/commands/`): fachada expuesta a JS, valida
  entrada y delega.
- **Lógica de dominio** (`src-tauri/src/sources/`, `parser/`): trait `Source`
  con implementaciones por cada sitio; parser puro sin I/O.
- **Datos** (`src-tauri/src/db/`): SQLite + repos con `sqlx`.
- **Red** (`reqwest`): cliente compartido inyectado a cada `Source`.

Añadir una nueva fuente = crear un módulo que implemente `Source` y añadirlo
a `init_state()` en `lib.rs`. El toggle del UI lo muestra automáticamente.

## Tests

```bash
cd app/src-tauri
cargo test
```

Hay tests unitarios en `parser::tests` que validan el parseo de los ejemplos
reales del usuario (`SceneryTR Design – LTFJ…`, `SiamFlight – RKSI…`, etc.).

## Notas legales

La app consulta las páginas públicas de SceneryAddons y Simplaza con un
User-Agent propio y respeta los enlaces de descarga que cada sitio provee.
No descarga contenido sin autorización del usuario: todos los enlaces se
abren en el navegador o se inician explícitamente por el usuario desde la UI.
