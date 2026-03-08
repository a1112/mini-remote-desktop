import assert from 'node:assert/strict';
import { AdaptiveTuner } from '../transport/adaptive-tuner.js';

const tuner = new AdaptiveTuner({
  initialBitrateKbps: 10000,
  minBitrateKbps: 2000,
  maxBitrateKbps: 15000,
  cooldownMs: 1000,
  targetFps: 60,
});

let patch = tuner.update({ nowMs: 1000, fps: 20, stallMs: 260, lossBurst: 0 });
assert.ok(patch, 'should tune down on low fps + stall');
assert.ok(patch.bitrateKbps < 10000);

patch = tuner.update({ nowMs: 1200, fps: 58, stallMs: 20, lossBurst: 0 });
assert.equal(patch, null, 'cooldown should block immediate retune');

patch = tuner.update({ nowMs: 2500, fps: 59, stallMs: 20, lossBurst: 0 });
assert.ok(patch, 'should tune up on stable high fps');

const beforeBackendDrop = tuner.bitrate;
patch = tuner.update({
  nowMs: 3600,
  fps: 58,
  stallMs: 30,
  lossBurst: 0,
  backend: {
    nackBurst: 12,
    queueDepth: 18,
  },
});
assert.ok(patch, 'should tune down on backend nack/queue pressure');
assert.ok(
  patch.bitrateKbps < beforeBackendDrop,
  'backend pressure should reduce bitrate',
);

let i = 0;
let now = 4000;
while (i++ < 20) {
  const p = tuner.update({ nowMs: now, fps: 5, stallMs: 300, lossBurst: 9 });
  now += 1100;
  if (p && p.bitrateKbps === 2000) break;
}
assert.equal(tuner.bitrate, 2000, 'should clamp to min bitrate');

console.log('adaptive-tuner tests passed');
