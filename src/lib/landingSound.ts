// (v6.2.56) Sonido de "toque de pista" (variante B: toque + reversa),
// sintetizado con Web Audio API — sin archivos ni copyright. Se reproduce
// cuando SimFleet detecta una descarga del navegador embebido de flightsim.to.

let ctx: AudioContext | null = null;

function ac(): AudioContext {
  if (!ctx) {
    const AC = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
    ctx = new AC();
  }
  if (ctx.state === "suspended") void ctx.resume();
  return ctx;
}

function noiseBuf(c: AudioContext, dur: number): AudioBuffer {
  const n = Math.floor(c.sampleRate * dur);
  const b = c.createBuffer(1, n, c.sampleRate);
  const d = b.getChannelData(0);
  for (let i = 0; i < n; i++) d[i] = Math.random() * 2 - 1;
  return b;
}

/** Chirrido resonante de las llantas (con vibrato + barrido de frecuencia). */
function squeal(
  c: AudioContext,
  t: number,
  o: { f0?: number; f1?: number; dur?: number; gain?: number; q?: number } = {},
) {
  const f0 = o.f0 ?? 1900,
    f1 = o.f1 ?? 1250,
    dur = o.dur ?? 0.5,
    gain = o.gain ?? 0.26,
    q = o.q ?? 20;
  const src = c.createBufferSource();
  src.buffer = noiseBuf(c, dur);
  const bp = c.createBiquadFilter();
  bp.type = "bandpass";
  bp.Q.value = q;
  bp.frequency.setValueAtTime(f0, t);
  bp.frequency.exponentialRampToValueAtTime(f1, t + dur);
  const lfo = c.createOscillator();
  lfo.type = "sine";
  lfo.frequency.value = 13;
  const lg = c.createGain();
  lg.gain.value = f0 * 0.05;
  lfo.connect(lg);
  lg.connect(bp.frequency);
  const g = c.createGain();
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(gain, t + 0.02);
  g.gain.setValueAtTime(gain, t + dur * 0.45);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  src.connect(bp);
  bp.connect(g);
  g.connect(c.destination);
  lfo.start(t);
  lfo.stop(t + dur);
  src.start(t);
  src.stop(t + dur);
}

/** Rodaje: ruido con paso-bajo que entra suave. */
function rumble(c: AudioContext, t: number, o: { dur?: number; gain?: number; cut?: number } = {}) {
  const dur = o.dur ?? 1.3,
    gain = o.gain ?? 0.2,
    cut = o.cut ?? 460;
  const src = c.createBufferSource();
  src.buffer = noiseBuf(c, dur);
  const lp = c.createBiquadFilter();
  lp.type = "lowpass";
  lp.frequency.value = cut;
  const g = c.createGain();
  g.gain.setValueAtTime(0.0001, t);
  g.gain.linearRampToValueAtTime(gain, t + 0.4);
  g.gain.setValueAtTime(gain, t + dur * 0.5);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  src.connect(lp);
  lp.connect(g);
  g.connect(c.destination);
  src.start(t);
  src.stop(t + dur);
}

/** Golpe de contacto (seno grave con caída rápida). */
function thump(c: AudioContext, t: number, o: { f?: number; dur?: number; gain?: number } = {}) {
  const f = o.f ?? 72,
    dur = o.dur ?? 0.2,
    gain = o.gain ?? 0.4;
  const osc = c.createOscillator();
  osc.type = "sine";
  osc.frequency.setValueAtTime(f, t);
  osc.frequency.exponentialRampToValueAtTime(f * 0.55, t + dur);
  const g = c.createGain();
  g.gain.setValueAtTime(gain, t);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(g);
  g.connect(c.destination);
  osc.start(t);
  osc.stop(t + dur);
}

/** Rugido de reversa (ruido paso-banda que sube y baja). */
function reverse(c: AudioContext, t: number, o: { dur?: number; gain?: number } = {}) {
  const dur = o.dur ?? 1.5,
    gain = o.gain ?? 0.24;
  const src = c.createBufferSource();
  src.buffer = noiseBuf(c, dur);
  const bp = c.createBiquadFilter();
  bp.type = "bandpass";
  bp.frequency.value = 520;
  bp.Q.value = 0.8;
  const g = c.createGain();
  g.gain.setValueAtTime(0.0001, t);
  g.gain.linearRampToValueAtTime(gain, t + 0.5);
  g.gain.setValueAtTime(gain, t + dur * 0.55);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  src.connect(bp);
  bp.connect(g);
  g.connect(c.destination);
  src.start(t);
  src.stop(t + dur);
}

/** Reproduce el sonido de aterrizaje (variante B: toque + reversa). */
export function playLandingSound() {
  try {
    const c = ac();
    const t = c.currentTime + 0.03;
    thump(c, t, { gain: 0.42 });
    squeal(c, t + 0.01, { f0: 1950, f1: 1300, dur: 0.5, gain: 0.26 });
    squeal(c, t + 0.14, { f0: 1750, f1: 1150, dur: 0.55, gain: 0.24, q: 18 });
    rumble(c, t + 0.05, { dur: 1.3, gain: 0.2 });
    reverse(c, t + 0.5, { dur: 1.5, gain: 0.24 });
  } catch {
    /* audio no disponible — ignorar */
  }
}
