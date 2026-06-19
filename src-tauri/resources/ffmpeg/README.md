# ffmpeg embebido (Best Landings)

Para que la grabación de "Best Landings" venga **incluida en el instalador**
(sin que el usuario tenga que descargar nada), coloca aquí el binario estático:

```
src-tauri/resources/ffmpeg/ffmpeg.exe
```

La forma fácil — desde la raíz del repo:

```powershell
pwsh scripts/fetch-ffmpeg.ps1
```

Eso descarga el build estático de ffmpeg y deja `ffmpeg.exe` en esta carpeta.
`tauri.conf.json` ya está configurado para bundlear esta carpeta (`resources/ffmpeg`
→ `ffmpeg/`), así que el siguiente `npm run tauri:build` lo incluirá.

Si `ffmpeg.exe` NO está presente al ejecutar la app, esta lo descarga
automáticamente a la carpeta de datos la primera vez que se necesita
(fallback silencioso — el usuario tampoco tiene que hacer clic).

> No commitees `ffmpeg.exe` al repo (es grande); está en `.gitignore`.
