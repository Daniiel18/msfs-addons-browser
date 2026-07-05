/**
 * (v6.2.34) Sonido de aviso corto vía Web Audio — sin assets. Se usa al
 * detectar el OFP del día para llamar la atención del usuario junto con
 * el foco de la ventana. Dos tonos ascendentes tipo "campana".
 */
export function playPreflightChime(): void {
  try {
    const Ctx =
      window.AudioContext ||
      (window as unknown as { webkitAudioContext?: typeof AudioContext })
        .webkitAudioContext;
    if (!Ctx) return;
    const ctx = new Ctx();
    const now = ctx.currentTime;
    const tone = (freq: number, start: number, dur: number) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = "sine";
      osc.frequency.value = freq;
      gain.gain.setValueAtTime(0.0001, now + start);
      gain.gain.exponentialRampToValueAtTime(0.22, now + start + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, now + start + dur);
      osc.connect(gain);
      gain.connect(ctx.destination);
      osc.start(now + start);
      osc.stop(now + start + dur + 0.02);
    };
    tone(660, 0, 0.18); // E5
    tone(880, 0.16, 0.28); // A5
    // Cierra el contexto tras sonar para no dejarlo colgado.
    setTimeout(() => ctx.close().catch(() => {}), 800);
  } catch {
    /* silencio si el navegador bloquea el audio */
  }
}
