import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';

const WINDOW_MS = 30_000;
const MAX_CALLBACKS = 8192;
const RTP_MODULUS = 4_294_967_296;
const finite = (value: unknown): number | null => typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= Number.MAX_SAFE_INTEGER ? value : null;
const integer = (value: unknown): number | null => { const n = finite(value); return n !== null && Number.isSafeInteger(n) ? n : null; };
const percentile = (values: number[], fraction: number): number | null => {
  if (!values.length) return null;
  const ordered = [...values].sort((a, b) => a - b);
  return ordered[Math.max(0, Math.ceil(fraction * ordered.length) - 1)] ?? null;
};

export type PresentationCadenceSummary = {
  elapsedMs: number | null;
  videoCallbacks: number;
  animationCallbacks: number;
  presentedFrameDelta: number | null;
  mediaTimeDeltaSeconds: number | null;
  rtpTimeDeltaMs: number | null;
  rafIntervalP50Ms: number | null;
  rafIntervalP95Ms: number | null;
  rafIntervalMaxMs: number | null;
  videoCallbackSpanMs: number | null;
  videoGapP95Ms: number | null;
  videoLateP95Ms: number | null;
  sampleLimitReached: 0 | 1;
};
type Diagnostic = { state: 'idle' | 'running' | 'complete' | 'cancelled' | 'unavailable'; summary: PresentationCadenceSummary | null };

class CadenceCollector {
  videoCallbacks = 0;
  animationCallbacks = 0;
  private rafPrevious: number | null = null;
  private videoFirst: number | null = null;
  private videoPrevious: number | null = null;
  private rafValid = true;
  private videoClockValid = true;
  private rafIntervals: number[] = [];
  private videoGaps: number[] = [];
  private videoLate: number[] = [];
  private presentedFirst: number | null = null;
  private presentedPrevious: number | null = null;
  private presentedValid = true;
  private mediaFirst: number | null = null;
  private mediaPrevious: number | null = null;
  private mediaValid = true;
  private rtpPrevious: number | null = null;
  private rtpTicks = 0;
  private rtpValid = true;

  animation(now: number) {
    this.animationCallbacks++;
    const n = finite(now);
    if (n === null || this.rafPrevious !== null && n <= this.rafPrevious) this.rafValid = false;
    else if (this.rafPrevious !== null) this.rafIntervals.push(n - this.rafPrevious);
    this.rafPrevious = n;
  }
  video(now: number, metadata: VideoFrameCallbackMetadata) {
    this.videoCallbacks++;
    const n = finite(now);
    if (n === null || this.videoPrevious !== null && n <= this.videoPrevious) this.videoClockValid = false;
    else if (this.videoPrevious !== null) this.videoGaps.push(n - this.videoPrevious);
    if (n !== null) this.videoFirst ??= n;
    this.videoPrevious = n;
    const expected = finite(metadata.expectedDisplayTime);
    if (n !== null && expected !== null) this.videoLate.push(Math.max(0, n - expected));
    const presented = integer(metadata.presentedFrames);
    if (presented === null || this.presentedPrevious !== null && presented < this.presentedPrevious) this.presentedValid = false;
    if (presented !== null) { this.presentedFirst ??= presented; this.presentedPrevious = presented; }
    const media = finite(metadata.mediaTime);
    if (media === null || this.mediaPrevious !== null && media < this.mediaPrevious) this.mediaValid = false;
    if (media !== null) { this.mediaFirst ??= media; this.mediaPrevious = media; }
    const rawRtp = integer(metadata.rtpTimestamp);
    const rtp = rawRtp !== null && rawRtp < RTP_MODULUS ? rawRtp : null;
    if (rtp === null) this.rtpValid = false;
    else if (this.rtpPrevious !== null) {
      const ticks = (rtp - this.rtpPrevious + RTP_MODULUS) % RTP_MODULUS;
      // A 30s span cannot cross half the 32-bit RTP clock. Treat backwards jumps as unknown.
      if (ticks >= RTP_MODULUS / 2) this.rtpValid = false;
      else this.rtpTicks += ticks;
    }
    this.rtpPrevious = rtp;
  }
  atLimit() { return this.videoCallbacks >= MAX_CALLBACKS || this.animationCallbacks >= MAX_CALLBACKS; }
  finish(elapsedMs: number): PresentationCadenceSummary {
    return {
      elapsedMs: finite(elapsedMs), videoCallbacks: this.videoCallbacks, animationCallbacks: this.animationCallbacks,
      presentedFrameDelta: this.presentedValid && this.presentedFirst !== null && this.presentedPrevious !== null && this.videoCallbacks > 1 ? finite(this.presentedPrevious - this.presentedFirst) : null,
      mediaTimeDeltaSeconds: this.mediaValid && this.mediaFirst !== null && this.mediaPrevious !== null && this.videoCallbacks > 1 ? finite(this.mediaPrevious - this.mediaFirst) : null,
      rtpTimeDeltaMs: this.rtpValid && this.videoCallbacks > 1 ? finite(this.rtpTicks / 90) : null,
      rafIntervalP50Ms: this.rafValid ? percentile(this.rafIntervals, .5) : null,
      rafIntervalP95Ms: this.rafValid ? percentile(this.rafIntervals, .95) : null,
      rafIntervalMaxMs: this.rafValid && this.rafIntervals.length ? Math.max(...this.rafIntervals) : null,
      videoCallbackSpanMs: this.videoClockValid && this.videoCallbacks > 1 && this.videoFirst !== null && this.videoPrevious !== null ? finite(this.videoPrevious - this.videoFirst) : null,
      videoGapP95Ms: this.videoClockValid ? percentile(this.videoGaps, .95) : null,
      videoLateP95Ms: this.videoClockValid ? percentile(this.videoLate, .95) : null,
      sampleLimitReached: this.atLimit() ? 1 : 0,
    };
  }
}

export function usePresentationCadenceDiagnostics(videoRef: RefObject<HTMLVideoElement | null>, enabled: boolean) {
  const video = videoRef.current;
  const supported = video !== null && typeof video.requestVideoFrameCallback === 'function' && typeof video.cancelVideoFrameCallback === 'function' && typeof window.requestAnimationFrame === 'function' && typeof window.cancelAnimationFrame === 'function';
  const [diagnostic, setDiagnostic] = useState<Diagnostic>({ state: 'idle', summary: null });
  const session = useRef<{ stop: () => void } | null>(null);
  useEffect(() => {
    setDiagnostic(current => ({ state: current.state === 'running' ? 'cancelled' : enabled && !supported ? 'unavailable' : 'idle', summary: null }));
    return () => { session.current?.stop(); session.current = null; };
  }, [enabled, supported, video]);

  const start = useCallback(() => {
    if (!enabled || !supported || video === null || videoRef.current !== video || session.current !== null) return;
    const startedAt = finite(performance.now());
    if (startedAt === null || startedAt > Number.MAX_SAFE_INTEGER - WINDOW_MS) { setDiagnostic({ state: 'unavailable', summary: null }); return; }
    const deadline = startedAt + WINDOW_MS;
    const collector = new CadenceCollector();
    let active = true;
    let videoRequest: number | null = null;
    let animationRequest: number | null = null;
    let timer: number | null = null;
    let visibilityListener: (() => void) | null = null;
    const stop = () => {
      active = false;
      if (videoRequest !== null) { try { video.cancelVideoFrameCallback(videoRequest); } catch { /* Diagnostic cleanup is best effort. */ } videoRequest = null; }
      if (animationRequest !== null) { window.cancelAnimationFrame(animationRequest); animationRequest = null; }
      if (timer !== null) { window.clearTimeout(timer); timer = null; }
      if (visibilityListener !== null) { document.removeEventListener('visibilitychange', visibilityListener); visibilityListener = null; }
    };
    const current = { stop };
    session.current = current;
    const finish = () => {
      if (!active) return;
      stop();
      if (session.current !== current) return;
      session.current = null;
      const endedAt = finite(performance.now());
      setDiagnostic({ state: 'complete', summary: collector.finish(endedAt === null || endedAt < startedAt ? Number.NaN : endedAt - startedAt) });
    };
    const unavailable = () => {
      if (!active) return;
      stop();
      if (session.current === current) { session.current = null; setDiagnostic({ state: 'unavailable', summary: null }); }
    };
    const inWindow = () => {
      if (!active || session.current !== current) return false;
      if (videoRef.current !== video) { stop(); session.current = null; setDiagnostic({ state: 'cancelled', summary: null }); return false; }
      const now = finite(performance.now());
      if (now === null || now < startedAt) { unavailable(); return false; }
      if (now >= deadline || collector.atLimit()) { finish(); return false; }
      return true;
    };
    const onVideo: VideoFrameRequestCallback = (now, metadata) => {
      if (!inWindow()) return;
      videoRequest = null;
      try {
        collector.video(now, metadata);
        if (collector.atLimit()) finish(); else videoRequest = video.requestVideoFrameCallback(onVideo);
      } catch { unavailable(); }
    };
    const onAnimation: FrameRequestCallback = now => {
      if (!inWindow()) return;
      animationRequest = null;
      try {
        collector.animation(now);
        if (collector.atLimit()) finish(); else animationRequest = window.requestAnimationFrame(onAnimation);
      } catch { unavailable(); }
    };
    setDiagnostic({ state: 'running', summary: null });
    visibilityListener = () => {
      if (!active || document.visibilityState === 'visible') return;
      stop();
      if (session.current === current) { session.current = null; setDiagnostic({ state: 'cancelled', summary: null }); }
    };
    document.addEventListener('visibilitychange', visibilityListener);
    try {
      timer = window.setTimeout(finish, WINDOW_MS);
      videoRequest = video.requestVideoFrameCallback(onVideo);
      animationRequest = window.requestAnimationFrame(onAnimation);
    } catch { unavailable(); }
  }, [enabled, supported, video, videoRef]);
  return { ...diagnostic, start, canStart: enabled && supported && diagnostic.state !== 'running' };
}
