import assert from 'node:assert/strict';
import { performance } from 'node:perf_hooks';
import { FrameReader } from '../transport/frame-wire.js';
import { AdaptiveTuner } from '../transport/adaptive-tuner.js';

function buildFrame(seq, txUs, payload) {
  const buf = new Uint8Array(20 + payload.length);
  const dv = new DataView(buf.buffer);
  dv.setUint32(0, payload.length, false);
  const seqBig = BigInt(seq);
  const txBig = BigInt(txUs);
  dv.setUint32(4, Number((seqBig >> 32n) & 0xffffffffn), false);
  dv.setUint32(8, Number(seqBig & 0xffffffffn), false);
  dv.setUint32(12, Number((txBig >> 32n) & 0xffffffffn), false);
  dv.setUint32(16, Number(txBig & 0xffffffffn), false);
  buf.set(payload, 20);
  return buf;
}

const parser = new FrameReader();
const frames = 2000;
const payload = new Uint8Array([0, 0, 0, 1, 0x65, 0x88, 0x84, 0x21]);
let parsed = 0;
const t0 = performance.now();
for (let i = 0; i < frames; i++) {
  const raw = buildFrame(i + 1, 1_000_000 + i * 16666, payload);
  const out = parser.push(raw);
  parsed += out.length;
}
const t1 = performance.now();
assert.equal(parsed, frames, 'all frames must be parsed');
const parseFps = frames / Math.max((t1 - t0) / 1000, 1e-9);
assert.ok(parseFps > 1000, `parser throughput too low: ${parseFps.toFixed(1)} fps`);

const tuner = new AdaptiveTuner({
  initialBitrateKbps: 10000,
  minBitrateKbps: 2000,
  maxBitrateKbps: 14000,
  cooldownMs: 100,
  targetFps: 60,
});

let up = 0;
let down = 0;
let now = Date.now();
for (let i = 0; i < 60; i++) {
  now += 120;
  const stable = i < 20;
  const degraded = i >= 20 && i < 45;
  const patch = tuner.update({
    nowMs: now,
    fps: stable ? 60 : degraded ? 25 : 58,
    stallMs: stable ? 20 : degraded ? 260 : 40,
    lossBurst: stable ? 0 : degraded ? 10 : 1,
  });
  if (!patch) continue;
  if (patch.bitrateKbps > 10000) up++;
  if (patch.bitrateKbps < 10000) down++;
}
assert.ok(down > 0, 'adaptive tuner should tune down under degradation');
assert.ok(up > 0, 'adaptive tuner should tune up after recovery');

console.log(`webtransport perf tests passed (parser=${parseFps.toFixed(0)}fps, tuneUp=${up}, tuneDown=${down})`);

