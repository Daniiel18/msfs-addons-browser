-- Persistencia del estado de descargas (v0.1.21).
--
-- Antes: el `DownloadManager` mantenía los jobs sólo en memoria
-- (`HashMap<String, DownloadJob>`). Si el usuario cerraba/mataba
-- la app a mitad de una descarga, el job se perdía aunque los
-- archivos parciales seguían en disco bajo `torrent_data_dir/job-{id}/`.
-- librqbit los habría resumido sin problema, pero el manager no
-- recordaba ningún job que retomar.
--
-- Ahora: cada cambio de estado del job se persiste aquí. Al
-- arrancar la app, `DownloadManager::restore_persisted()` lee la
-- tabla, repopula el HashMap y re-lanza `run_torrent` para los
-- jobs no-terminales. librqbit hace el resume al ver los archivos
-- parciales en `job-{id}/` (gracias a `overwrite: true`).
--
-- Nota: la tabla `downloads` original (migración 001) era un stub
-- de "historial de descargas" que nunca llegó a usarse — schema
-- incompatible con `DownloadJob` (PK INTEGER vs TEXT UUID). La
-- dejamos en paz; esta tabla es independiente.

CREATE TABLE IF NOT EXISTS download_jobs (
    id              TEXT PRIMARY KEY,
    addon_id        TEXT NOT NULL,
    addon_title     TEXT NOT NULL,
    source          TEXT NOT NULL,
    -- 'torrent' | 'mirror' | 'direct' (string para no acoplar al enum)
    method_kind     TEXT NOT NULL,
    method_name     TEXT NOT NULL,
    -- URL original o magnet — para torrents que se resolvieron, puede
    -- ya ser un `magnet:?...` (ahorra los 12s del Read-First gate al
    -- resumir).
    url             TEXT NOT NULL,
    -- 'queued' | 'resolving' | 'downloading' | 'paused' |
    -- 'installing' | 'completed' | 'cancelled' | 'error'
    phase           TEXT NOT NULL,
    bytes_total     INTEGER NOT NULL DEFAULT 0,
    bytes_done      INTEGER NOT NULL DEFAULT 0,
    speed_bps       INTEGER NOT NULL DEFAULT 0,
    eta_seconds     INTEGER,
    message         TEXT,
    error           TEXT,
    install_path    TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_download_jobs_phase ON download_jobs(phase);
CREATE INDEX IF NOT EXISTS idx_download_jobs_created_at ON download_jobs(created_at);
