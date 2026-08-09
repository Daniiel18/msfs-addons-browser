//! Identificadores privados de cuenta (hashes SHA-256).
//!
//! Este repositorio es PÚBLICO, así que los correos de las cuentas que
//! usan las funciones «privadas» (allowlist del cloud sync + detección
//! del dueño para filtros de propiedad / identidad de piloto) NO se
//! guardan en claro: solo su hash SHA-256, del correo normalizado
//! (trim + minúsculas). La comparación por igualdad de hash preserva
//! exactamente el comportamiento del allowlist / detección anterior,
//! sin exponer las direcciones. Para autorizar una cuenta nueva basta
//! con añadir su hash aquí y recompilar.

use sha2::{Digest, Sha256};

/// SHA-256 (hex en minúsculas) del correo normalizado (trim + lowercase).
pub fn email_hash(email: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(email.trim().to_lowercase().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Cuentas autorizadas a sincronizar con Google Drive (cloud sync).
const SYNC_WHITELIST_SHA256: &[&str] = &[
    "49ea667c377c19574300ebe1945be5c0afd3aee066a48a0f712164537a6bf5a6",
    "e556446ffdc7dda7b2b6c764e5792bf1a5c106b5433aeada09aaf5698fbc8220",
    "f6640e6841741ce28526326e9ac6a75f72008d4575a12bb426ecfe4a61898670",
];

/// Cuentas del dueño (filtros de propiedad de OFPs / identidad de piloto).
const OWNER_SHA256: &[&str] = &[
    "e556446ffdc7dda7b2b6c764e5792bf1a5c106b5433aeada09aaf5698fbc8220",
    "f6640e6841741ce28526326e9ac6a75f72008d4575a12bb426ecfe4a61898670",
];

/// ¿El correo está autorizado para el cloud sync? (equivalente al viejo
/// `WHITELIST_EMAILS.contains`, pero comparando hashes).
pub fn is_sync_authorized(email: &str) -> bool {
    let h = email_hash(email);
    SYNC_WHITELIST_SHA256.contains(&h.as_str())
}

/// ¿El correo pertenece al dueño? (equivalente al viejo `OWNER_EMAILS`
/// / al substring `contains("jose.daniel")`).
pub fn is_owner(email: &str) -> bool {
    let h = email_hash(email);
    OWNER_SHA256.contains(&h.as_str())
}
