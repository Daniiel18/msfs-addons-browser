# Resources bundleados

Archivos que se incluyen en el `setup.exe` y la app puede leer en
runtime via `app.path().resolve(name, BaseDirectory::Resource)`.

## `airports.csv` — dataset de aeropuertos OurAirports

Origen: <https://davidmegginson.github.io/ourairports-data/airports.csv>

Para que la app funcione **sin internet** desde la primera apertura:

1. Antes del `cargo tauri build` (o `npm run tauri build`), descargar
   el CSV y guardarlo aquí como `airports.csv`.

   ```powershell
   curl -L -o app/src-tauri/resources/airports.csv `
        https://davidmegginson.github.io/ourairports-data/airports.csv
   ```

2. Construir el setup.exe normalmente. Tauri lo bundleará gracias a
   `bundle.resources` en `tauri.conf.json`.

3. En la primera apertura, `airports::ensure_dataset_with_app` lo lee
   del bundle y popula la tabla `airports`. Sin red. ~7 MB → 30 k
   filas → ~2-3 segundos.

Si no hay archivo aquí cuando se construye el setup, la app sigue
funcionando — sólo que la primera apertura intenta descargarlo de
internet. Las siguientes (con cache local) funcionan offline.
