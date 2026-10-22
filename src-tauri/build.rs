use std::fs;
use std::path::Path;

fn main() {
    // (v3.5.0 F2) Re-compilar cuando cambian las migraciones. `sqlx::migrate!`
    // embebe los archivos `.sql` en el binario en tiempo de compilación, pero
    // Cargo no sabe que tiene que invalidar el incremental cache cuando un
    // archivo nuevo aparece en `migrations/`. Sin esto, agregar `019_*.sql`
    // y reconstruir devuelve el binario con la lista de migraciones VIEJA
    // (porque el .rlib cacheado todavía es válido desde el POV de Cargo) y
    // la nueva columna nunca se crea en producción. El bug manifestó como
    // «no such column: external_id» tras agregar la migración 019.
    println!("cargo:rerun-if-changed=migrations");

    // (v3.1.1) Inyecta secretos OAuth desde `secrets.local.toml` al
    // entorno de compilación. El archivo está gitignored para que
    // GitHub secret scanning no bloquee el push. La build local lo
    // lee y exporta los valores como env vars; `option_env!` en
    // `cloud_sync.rs` los recoge y los embebe en el binario.
    //
    // Formato esperado (TOML simple, una clave por línea):
    //   SIMFLEET_GOOGLE_CLIENT_ID = "xxx.apps.googleusercontent.com"
    //   SIMFLEET_GOOGLE_CLIENT_SECRET = "GOCSPX-xxx"
    //
    // Si el archivo no existe, no es fatal — los `option_env!` quedan
    // en `None` y la app rechaza el OAuth con un mensaje claro.
    let secrets_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("secrets.local.toml");
    println!("cargo:rerun-if-changed=secrets.local.toml");
    if let Ok(content) = fs::read_to_string(&secrets_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Parser minimal: KEY = "VALUE" (con o sin espacios y
            // con o sin comillas). No es TOML completo — sólo lo que
            // necesitamos para 2-3 secrets.
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if key.is_empty() || value.is_empty() {
                continue;
            }
            // (v8.3 · Etapa 6.2a) Las claves de OpenSky van CIFRADAS en el
            // binario (`SIMFLEET_OPENSKY_*_X`), nunca en texto plano. Las
            // descifra `opensky::embedded_credentials`.
            if key.starts_with("SIMFLEET_OPENSKY_") {
                println!("cargo:rustc-env={}_X={}", key, seal(value));
            } else {
                println!("cargo:rustc-env={}={}", key, value);
            }
        }
    }

    tauri_build::build();
}

/// Cifra `value` con un flujo XOR (splitmix64) a partir de una semilla
/// aleatoria distinta en cada build. Devuelve `semilla:cifrado` en hex.
/// La semilla viaja con el dato, así que esto sólo evita que el secreto se
/// lea a simple vista en el `.exe`: no protege contra alguien decidido.
/// Debe coincidir con `opensky::unseal`.
fn seal(value: &str) -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    h.write_u128(nanos);
    let seed = h.finish();
    let mut state = seed;
    let cipher: String = value
        .bytes()
        .map(|b| format!("{:02x}", b ^ (splitmix64(&mut state) >> 24) as u8))
        .collect();
    format!("{seed:016x}:{cipher}")
}

fn splitmix64(s: &mut u64) -> u64 {
    *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
