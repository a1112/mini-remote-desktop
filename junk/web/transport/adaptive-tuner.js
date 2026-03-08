export class AdaptiveTuner {
  constructor(opts = {}) {
    this.minBitrate = opts.minBitrateKbps ?? 1500;
    this.maxBitrate = opts.maxBitrateKbps ?? 24000;
    this.stepUp = opts.stepUpKbps ?? 800;
    this.stepDown = opts.stepDownKbps ?? 1200;
    this.cooldownMs = opts.cooldownMs ?? 2000;
    this.targetFps = opts.targetFps ?? 60;
    this.bitrate = opts.initialBitrateKbps ?? 12000;
    this.lastTuneAt = 0;
  }

  update(sample) {
    const now = sample.nowMs ?? Date.now();
    if (this.lastTuneAt && now - this.lastTuneAt < this.cooldownMs) {
      return null;
    }
    const fps = sample.fps ?? 0;
    const stallMs = sample.stallMs ?? 0;
    const lossBurst = sample.lossBurst ?? 0;
    const nackBurst = sample.backend?.nackBurst ?? 0;
    const queueDepth = sample.backend?.queueDepth ?? 0;

    let next = this.bitrate;
    if (
      stallMs > 200 ||
      lossBurst >= 8 ||
      nackBurst >= 8 ||
      queueDepth >= 12 ||
      fps < this.targetFps * 0.65
    ) {
      next = Math.max(this.minBitrate, this.bitrate - this.stepDown);
    } else if (
      fps > this.targetFps * 0.9 &&
      stallMs < 80 &&
      lossBurst <= 2 &&
      nackBurst <= 2 &&
      queueDepth <= 3
    ) {
      next = Math.min(this.maxBitrate, this.bitrate + this.stepUp);
    }

    if (next === this.bitrate) return null;
    this.bitrate = next;
    this.lastTuneAt = now;
    return {
      bitrateKbps: next,
    };
  }
}
