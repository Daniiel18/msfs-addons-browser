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
            if !key.is_empty() && !value.is_empty() {
                println!("cargo:rustc-env={}={}", key, value);
            }
        }
    }

    tauri_build::build();
}
