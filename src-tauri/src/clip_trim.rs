//! (8.4.0) Recorte del clip de aterrizaje a [toque − 30 s, toque + 10 s].
//!
//! La captura graba en disco toda la aproximación (desde que se arma) y al
//! guardar nos quedamos sólo con ese tramo. Copiar las muestras tal cual no
//! sirve: el encoder H.264 por hardware de Windows emite UN solo keyframe al
//! principio (medido en un clip real: 1 IDR y 9 421 frames P en 314 s), así
//! que el vídeo sólo se puede decodificar desde el segundo 0. Por eso el
//! tramo se recodifica con el `MediaTranscoder` de Windows (el mismo stack
//! que graba, por hardware y al mismo bitrate): corta al fotograma exacto y
//! tarda ~10 s para una aproximación de 5 min (decodifica desde el inicio a
//! ~30× tiempo real y codifica sólo los 40 s del clip).
#![cfg(windows)]

use std::path::Path;

use anyhow::{anyhow, bail, Context};
use windows::core::HSTRING;
use windows::Foundation::TimeSpan;
use windows::Media::MediaProperties::{MediaEncodingProfile, VideoEncodingQuality};
use windows::Media::Transcoding::MediaTranscoder;
use windows::Storage::StorageFile;

/// Unidades de `TimeSpan` (100 ns) por segundo.
const TICKS: f64 = 10_000_000.0;
/// Por debajo de esto no merece la pena recortar (ni recodificar).
const SLACK_S: f64 = 0.5;

/// Qué hay que cortar de una grabación de `total_s` para quedarse con
/// [`from_s`, `to_s`]: `(inicio, fin)` en segundos, `None` en cada extremo
/// que no hace falta tocar. `None` del todo = la grabación ya es el tramo.
fn cut(from_s: f64, to_s: f64, total_s: f64) -> Option<(Option<f64>, Option<f64>)> {
    let start = (from_s > SLACK_S && from_s < total_s).then_some(from_s);
    let stop = (to_s < total_s - SLACK_S && to_s > from_s.max(0.0)).then_some(to_s);
    (start.is_some() || stop.is_some()).then_some((start, stop))
}

/// `StorageFile` de una ruta local (WinRT no acepta el prefijo `\\?\`).
fn storage_file(path: &Path) -> anyhow::Result<StorageFile> {
    let abs = std::fs::canonicalize(path).with_context(|| format!("{}", path.display()))?;
    let s = abs.to_string_lossy();
    let plain = match s.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => s.strip_prefix(r"\\?\").unwrap_or(&s).to_string(),
    };
    StorageFile::GetFileFromPathAsync(&HSTRING::from(plain))
        .and_then(|op| op.get())
        .map_err(|e| anyhow!("{}: {e}", path.display()))
}

/// Duración del vídeo según Windows (propiedades del archivo).
fn duration_s(file: &StorageFile) -> anyhow::Result<f64> {
    let props = file.Properties()?.GetVideoPropertiesAsync()?.get()?;
    Ok(props.Duration()?.Duration as f64 / TICKS)
}

/// Perfil de salida = el de la grabación (H.264 + AAC, mismas dimensiones,
/// fps y bitrate), para que el clip recortado se vea igual que el original.
fn profile_like(src: &StorageFile) -> anyhow::Result<MediaEncodingProfile> {
    let source = MediaEncodingProfile::CreateFromFileAsync(src)?.get()?;
    let profile = MediaEncodingProfile::CreateMp4(VideoEncodingQuality::HD1080p)?;
    let (sv, v) = (source.Video()?, profile.Video()?);
    v.SetWidth(sv.Width()?)?;
    v.SetHeight(sv.Height()?)?;
    v.SetBitrate(sv.Bitrate()?)?;
    v.FrameRate()?.SetNumerator(sv.FrameRate()?.Numerator()?)?;
    v.FrameRate()?.SetDenominator(sv.FrameRate()?.Denominator()?)?;
    match source.Audio() {
        Ok(sa) => {
            let a = profile.Audio()?;
            a.SetSampleRate(sa.SampleRate()?)?;
            a.SetChannelCount(sa.ChannelCount()?)?;
            if sa.Bitrate()? > 0 {
                a.SetBitrate(sa.Bitrate()?)?;
            }
        }
        // Grabación sin audio (dispositivo no compatible): clip sin audio.
        Err(_) => profile.SetAudio(None)?,
    }
    Ok(profile)
}

/// Escribe en `dst` el tramo [`from_s`, `to_s`] (segundos desde el inicio de
/// `src`). Devuelve la duración del clip, o `None` si la grabación ya era el
/// tramo pedido (no se escribe nada: basta con mover `src`). Bloqueante.
pub fn trim(src: &Path, dst: &Path, from_s: f64, to_s: f64) -> anyhow::Result<Option<f64>> {
    let source = storage_file(src)?;
    let total = duration_s(&source).context("duración de la grabación")?;
    let Some((start, stop)) = cut(from_s, to_s, total) else {
        return Ok(None);
    };
    let profile = profile_like(&source).context("perfil de la grabación")?;

    std::fs::File::create(dst).with_context(|| format!("crear {}", dst.display()))?;
    let target = storage_file(dst)?;
    let tr = MediaTranscoder::new()?;
    tr.SetHardwareAccelerationEnabled(true)?;
    // Ojo: `TrimStopTime` es la POSICIÓN final (no "cuánto quitar del final").
    if let Some(s) = start {
        tr.SetTrimStartTime(TimeSpan { Duration: (s * TICKS) as i64 })?;
    }
    if let Some(s) = stop {
        tr.SetTrimStopTime(TimeSpan { Duration: (s * TICKS) as i64 })?;
    }
    let prepared = tr.PrepareFileTranscodeAsync(&source, &target, &profile)?.get()?;
    if !prepared.CanTranscode()? {
        bail!("Windows no puede recortar el clip ({:?})", prepared.FailureReason()?);
    }
    prepared.TranscodeAsync()?.get()?;

    let expected = stop.unwrap_or(total) - start.unwrap_or(0.0);
    Ok(Some(duration_s(&target).unwrap_or(expected)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_only_what_is_outside_the_window() {
        // Aproximación de 5 min: toque a los 302 s → [272, 312] de 314.
        assert_eq!(cut(272.0, 312.0, 314.0), Some((Some(272.0), Some(312.0))));
        // Armado tarde (toque a los 20 s): no hay 30 s previos → sólo el final.
        assert_eq!(cut(-10.0, 30.0, 33.0), Some((None, Some(30.0))));
        // El final ya está dentro del margen: sólo el inicio.
        assert_eq!(cut(100.0, 140.0, 140.2), Some((Some(100.0), None)));
        // La grabación ya es el tramo: nada que hacer.
        assert_eq!(cut(0.2, 40.0, 40.1), None);
        // Ventana fuera de la grabación (reloj raro): nada que cortar.
        assert_eq!(cut(500.0, 540.0, 300.0), None);
    }

    /// Recorte manual de un clip real (sólo lo LEE; escribe en `CLIP_OUT`):
    /// `CLIP=<mp4> CLIP_OUT=<mp4> CLIP_FROM=272 CLIP_TO=312 cargo test --lib
    /// clip_trim_probe -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn clip_trim_probe() {
        let src = std::path::PathBuf::from(std::env::var("CLIP").expect("CLIP"));
        let out = std::path::PathBuf::from(std::env::var("CLIP_OUT").expect("CLIP_OUT"));
        let num = |k: &str| std::env::var(k).ok().and_then(|s| s.parse::<f64>().ok());
        let from = num("CLIP_FROM").unwrap_or(10.0);
        let to = num("CLIP_TO").unwrap_or(from + 40.0);
        let t0 = std::time::Instant::now();
        let d = trim(&src, &out, from, to).unwrap();
        eprintln!("recorte {from}..{to} → {d:?} s en {:?}", t0.elapsed());
    }
}
