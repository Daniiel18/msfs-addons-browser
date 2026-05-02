//! Download orchestration.
//!
//! Every user action ("download this mirror", "grab this torrent")
//! becomes a [`DownloadJob`] managed by [`manager::DownloadManager`].
//! State transitions and progress are pushed to the frontend via the
//! Tauri event channel [`manager::EVENT_JOB_UPDATE`] so the UI is a
//! pure view over the backend truth.

pub mod manager;
pub mod torrent;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadPhase {
    /// Waiting for its turn in the queue.
    Queued,
    /// Asking the mirror for a magnet / redirect target.
    Resolving,
    /// Bytes are flowing.
    Downloading,
    /// Torrent is paused by the user; can be resumed.
    Paused,
    /// Archive is being extracted + copied to Community.
    Installing,
    /// Install complete.
    Completed,
    /// User cancelled.
    Cancelled,
    /// Terminal error — see `error`.
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadJob {
    pub id: String,
    pub addon_id: String,
    pub addon_title: String,
    pub source: String,
    /// "torrent" | "mirror" | "direct"
    pub method_kind: String,
    pub method_name: String,
    pub url: String,
    pub phase: DownloadPhase,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub speed_bps: u64,
    pub eta_seconds: Option<u64>,
    /// Human-readable status line for the UI (overrides phase label when set).
    pub message: Option<String>,
    /// Terminal error message when `phase == Error`.
    pub error: Option<String>,
    /// Final install path when `phase == Completed` (torrent flow only).
    pub install_path: Option<String>,
    pub created_at: String,
}
