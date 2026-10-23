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
        // Parser minimal: KEY = "VALUE" (con o sin espacios y con o sin
        // comillas). No es TOML completo — sólo lo que necesitamos.
        let pairs: Vec<(String, String)> = content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').trim_matches('\x27').to_string()))
            .filter(|(k, v)| !k.is_empty() && !v.is_empty())
            .collect();
        // (v8.3) Código de activación: cifra las claves de OpenSky y NO se
        // incrusta. Sin él (o sin escribirlo en Ajustes) no se pueden usar.
        let activation = pairs
            .iter()
            .find(|(k, _)| k == "SIMFLEET_OPENSKY_ACTIVATION")
            .map(|(_, v)| v.clone());
        for (key, value) in &pairs {
            if key == "SIMFLEET_OPENSKY_ACTIVATION" {
                continue;
            }
            // (v8.3 · Etapa 6.2a) Las claves de OpenSky van CIFRADAS en el
            // binario (`SIMFLEET_OPENSKY_*_X`), nunca en texto plano. Las
            // descifra `opensky::embedded_credentials`.
            if key.starts_with("SIMFLEET_OPENSKY_") {
                println!("cargo:rustc-env={}_X={}", key, seal(value, activation.as_deref()));
            } else {
                println!("cargo:rustc-env={}={}", key, value);
            }
        }
    }

    tauri_build::build();
}

/// Cifra `value` con un flujo XOR (splitmix64).
///
/// - Sin código de activación: semilla aleatoria por build, que viaja con el
///   dato (`semilla:cifrado`). Sólo evita leerlo a simple vista en el `.exe`.
/// - Con código (v8.3): la clave sale del código (estirado 20.000 vueltas con
///   una sal aleatoria) y el código NO va en el binario: `v2:sal:check:cifrado`
///   (`check` = FNV-1a del texto, para saber si el código es correcto).
///
/// Debe coincidir con `opensky::unseal`.
fn seal(value: &str, activation: Option<&str>) -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    h.write_u128(nanos);
    let random = h.finish();
    let xor = |mut state: u64| -> String {
        value
            .bytes()
            .map(|b| format!("{:02x}", b ^ (splitmix64(&mut state) >> 24) as u8))
            .collect()
    };
    match activation {
        None => format!("{random:016x}:{}", xor(random)),
        Some(code) => {
            let key = stretch(code, random);
            format!("v2:{random:016x}:{:016x}:{}", fnv64(value.as_bytes()), xor(key))
        }
    }
}

/// Código de activación normalizado: sólo letras y dígitos, en mayúsculas.
fn normalize_code(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase()
}

/// Clave a partir del código (+ sal), estirada para encarecer la fuerza bruta.
fn stretch(code: &str, salt: u64) -> u64 {
    let code = normalize_code(code);
    let mut s = salt;
    for _ in 0..20_000 {
        for b in code.bytes() {
            s = (s ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        splitmix64(&mut s);
    }
    s
}

fn fnv64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h = (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn splitmix64(s: &mut u64) -> u64 {
    *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
