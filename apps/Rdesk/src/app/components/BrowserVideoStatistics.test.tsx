import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BrowserVideoStatistics } from './BrowserVideoStatistics';

function videoFixture() {
  const video = document.createElement('video');
  Object.defineProperties(video, {
    videoWidth: { configurable: true, value: 2560 },
    videoHeight: { configurable: true, value: 1440 },
    currentTime: { configurable: true, value: 12.5 },
    readyState: { configurable: true, value: 4 },
    paused: { configurable: true, value: false },
    ended: { configurable: true, value: false },
  });
  const quality = vi.fn(() => ({
    totalVideoFrames: 100, droppedVideoFrames: 2, corruptedVideoFrames: 0,
    creationTime: 0,
  }));
  Object.defineProperty(video, 'getVideoPlaybackQuality', { configurable: true, value: quality });
  return { videoRef: { current: video }, quality };
}

function panel() {
  const details = screen.getByText('播放统计').closest('details');
  if (!details) throw new Error('播放统计 must be a details disclosure');
  return details;
}

function toggle(open: boolean) {
  const details = panel();
  act(() => {
    details.open = open;
    fireEvent(details, new Event('toggle'));
  });
  return details;
}

describe('browser video playback statistics', () => {
  let monotonicMs = 1000;
  beforeEach(() => {
    vi.useFakeTimers();
    monotonicMs = 1000;
    vi.spyOn(performance, 'now').mockImplementation(() => monotonicMs);
  });
  afterEach(() => {
    cleanup();
    vi.clearAllTimers();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('does not sample collapsed or unready and exposes real deltas only while expanded', () => {
    const { videoRef, quality } = videoFixture();
    const view = render(<BrowserVideoStatistics videoRef={videoRef} ready={false} />);
    expect(panel().open).toBe(false);
    act(() => vi.advanceTimersByTime(3000));
    expect(quality).not.toHaveBeenCalled();
    toggle(true);
    act(() => vi.advanceTimersByTime(2000));
    expect(quality).not.toHaveBeenCalled();

    view.rerender(<BrowserVideoStatistics videoRef={videoRef} ready />);
    expect(quality).toHaveBeenCalledTimes(1);
    expect(panel()).toHaveAttribute('data-total-video-frames', '100');
    expect(panel()).toHaveAttribute('data-dropped-video-frames', '2');
    expect(panel()).toHaveAttribute('data-corrupted-video-frames', '0');
    expect(panel()).toHaveAttribute('data-sampled-at-ms', '1000');
    expect(panel()).toHaveAttribute('data-video-width', '2560');
    expect(panel()).toHaveAttribute('data-video-height', '1440');
    expect(panel()).toHaveAttribute('data-current-time', '12.5');
    expect(panel()).toHaveAttribute('data-ready-state', '4');
    expect(panel()).toHaveAttribute('data-paused', 'false');
    expect(panel()).toHaveAttribute('data-ended', 'false');
    expect(panel()).not.toHaveAttribute('data-frame-count-per-second');

    monotonicMs = 2000;
    quality.mockReturnValue({ totalVideoFrames: 160, droppedVideoFrames: 3, corruptedVideoFrames: 1, creationTime: 0 });
    act(() => vi.advanceTimersByTime(1000));
    expect(panel()).toHaveAttribute('data-total-video-frames', '160');
    expect(panel()).toHaveAttribute('data-total-frame-delta', '60');
    expect(panel()).toHaveAttribute('data-dropped-frame-delta', '1');
    expect(panel()).toHaveAttribute('data-corrupted-frame-delta', '1');
    expect(panel()).toHaveAttribute('data-elapsed-ms', '1000');
    expect(panel()).toHaveAttribute('data-frame-count-per-second', '60');
    expect(screen.getByLabelText('帧计数/秒')).toHaveTextContent('60.00');
    expect(screen.getByText(/不代表独立源画面、RTP 帧数或网络延迟/)).toBeInTheDocument();

    toggle(false);
    act(() => vi.advanceTimersByTime(3000));
    expect(quality).toHaveBeenCalledTimes(2);
    expect(panel()).not.toHaveAttribute('data-sampled-at-ms');
  });

  it('marks unsupported frame counting unavailable without fabricating zero counters', () => {
    const { videoRef } = videoFixture();
    Object.defineProperty(videoRef.current, 'getVideoPlaybackQuality', { configurable: true, value: undefined });
    render(<BrowserVideoStatistics videoRef={videoRef} ready />);
    toggle(true);
    expect(panel()).toHaveAttribute('data-quality-status', 'unsupported');
    expect(screen.getByText(/当前浏览器不支持播放帧计数/)).toBeInTheDocument();
    expect(screen.getByLabelText('总帧数')).toHaveTextContent('不可用');
    expect(screen.getByLabelText('丢帧数')).toHaveTextContent('不可用');
    expect(panel()).not.toHaveAttribute('data-total-video-frames');
    expect(panel()).not.toHaveAttribute('data-frame-count-per-second');
    expect(panel()).toHaveAttribute('data-video-width', '2560');
  });

  it('does not report FPS across a counter reset, invalid counts or a nonpositive interval', () => {
    const { videoRef, quality } = videoFixture();
    render(<BrowserVideoStatistics videoRef={videoRef} ready />);
    toggle(true);
    monotonicMs = 2000;
    quality.mockReturnValue({ totalVideoFrames: 10, droppedVideoFrames: 0, corruptedVideoFrames: 0, creationTime: 0 });
    act(() => vi.advanceTimersByTime(1000));
    expect(panel()).toHaveAttribute('data-total-video-frames', '10');
    expect(panel()).not.toHaveAttribute('data-total-frame-delta');
    expect(panel()).not.toHaveAttribute('data-frame-count-per-second');
    expect(screen.getByLabelText('帧计数/秒')).toHaveTextContent('不可用');

    quality.mockReturnValue({ totalVideoFrames: 70, droppedVideoFrames: 0, corruptedVideoFrames: 0, creationTime: 0 });
    act(() => vi.advanceTimersByTime(1000)); // The monotonic clock has not advanced.
    expect(panel()).not.toHaveAttribute('data-frame-count-per-second');
    monotonicMs = 3000;
    quality.mockReturnValue({ totalVideoFrames: Number.NaN, droppedVideoFrames: -1, corruptedVideoFrames: 0, creationTime: 0 });
    act(() => vi.advanceTimersByTime(1000));
    expect(panel()).toHaveAttribute('data-quality-status', 'invalid');
    expect(panel()).not.toHaveAttribute('data-total-video-frames');
    expect(panel()).not.toHaveAttribute('data-frame-count-per-second');
    expect(panel().outerHTML).not.toMatch(/NaN|Infinity|data-[\w-]+="-\d/);

    monotonicMs = 4000;
    quality.mockReturnValue({ totalVideoFrames: 130, droppedVideoFrames: 0, corruptedVideoFrames: 0, creationTime: 0 });
    act(() => vi.advanceTimersByTime(1000));
    expect(panel()).not.toHaveAttribute('data-frame-count-per-second');
    monotonicMs = 5000;
    quality.mockReturnValue({ totalVideoFrames: 190, droppedVideoFrames: 0, corruptedVideoFrames: 0, creationTime: 0 });
    act(() => vi.advanceTimersByTime(1000));
    expect(panel()).toHaveAttribute('data-frame-count-per-second', '60');
  });

  it('stops when readiness is lost or unmounted and restarts with a fresh baseline', () => {
    const { videoRef, quality } = videoFixture();
    const view = render(<BrowserVideoStatistics videoRef={videoRef} ready />);
    toggle(true);
    expect(quality).toHaveBeenCalledTimes(1);
    view.rerender(<BrowserVideoStatistics videoRef={videoRef} ready={false} />);
    act(() => vi.advanceTimersByTime(3000));
    expect(quality).toHaveBeenCalledTimes(1);
    expect(panel()).not.toHaveAttribute('data-sampled-at-ms');
    monotonicMs = 5000;
    quality.mockReturnValue({ totalVideoFrames: 340, droppedVideoFrames: 2, corruptedVideoFrames: 0, creationTime: 0 });
    view.rerender(<BrowserVideoStatistics videoRef={videoRef} ready />);
    expect(quality).toHaveBeenCalledTimes(2);
    expect(panel()).not.toHaveAttribute('data-frame-count-per-second');
    view.unmount();
    act(() => vi.advanceTimersByTime(3000));
    expect(quality).toHaveBeenCalledTimes(2);
  });
});


describe('inbound video statistics', () => {
  let clock = 1000;
  const raw = (n = 0) => ({ framesReceived: 100 + n, framesDecoded: 90 + n, framesDropped: 2,
    packetsLost: 0, totalDecodeTime: 0.5 + n / 100, bytesReceived: 100000 + n * 1000,
    jitter: 0.004, jitterBufferDelay: 0.8 + n / 50, jitterBufferEmittedCount: 90 + n });
  beforeEach(() => { vi.useFakeTimers(); clock = 1000; vi.spyOn(performance, 'now').mockImplementation(() => clock); });
  afterEach(() => { cleanup(); vi.clearAllTimers(); vi.useRealTimers(); vi.restoreAllMocks(); });

  it('derives only consecutive counters and resets on rollback or nonmonotonic time', async () => {
    const module = await import('./BrowserVideoStatistics');
    const rates = (module as unknown as { inboundVideoRates: (previous: unknown, current: unknown) => unknown }).inboundVideoRates;
    const first = { sampledAtMs: 1000, counters: raw() };
    const second = { sampledAtMs: 2000, counters: raw(60) };
    expect(rates(null, first)).toBeNull();
    expect(rates(first, second)).toEqual({ receiveFps: 60, decodeFps: 60, decodeMs: expect.closeTo(10, 8), jitterBufferMs: 20, receiveMbps: 0.48 });
    expect(rates(second, { sampledAtMs: 3000, counters: raw(2) })).toBeNull();
    expect(rates(first, { ...second, sampledAtMs: 1000 })).toBeNull();
    expect(rates(first, { ...second, sampledAtMs: 500 })).toBeNull();
    expect(rates(first, { ...second, counters: { ...raw(60), jitter: 0.001 } })).toEqual(
      { receiveFps: 60, decodeFps: 60, decodeMs: expect.closeTo(10, 8), jitterBufferMs: 20, receiveMbps: 0.48 });
    expect(rates(first, { sampledAtMs: 2000, counters: raw() })).toEqual(
      { receiveFps: 0, decodeFps: 0, decodeMs: null, jitterBufferMs: null, receiveMbps: 0 });
  });

  it('samples only expanded ready, does not overlap, and ignores late data after collapse or unmount', async () => {
    const { videoRef } = videoFixture();
    let resolve!: (value: unknown) => void;
    const sample = vi.fn(() => new Promise(done => { resolve = done; }));
    const Component = BrowserVideoStatistics as unknown as React.ComponentType<{videoRef: typeof videoRef; ready: boolean; sampleInboundVideoStats: typeof sample}>;
    const view = render(<Component videoRef={videoRef} ready={false} sampleInboundVideoStats={sample} />);
    toggle(true); await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(sample).not.toHaveBeenCalled();
    view.rerender(<Component videoRef={videoRef} ready sampleInboundVideoStats={sample} />);
    expect(sample).toHaveBeenCalledTimes(1);
    await act(async () => vi.advanceTimersByTimeAsync(3000));
    expect(sample).toHaveBeenCalledTimes(1);
    toggle(false);
    await act(async () => resolve(raw()));
    expect(panel()).not.toHaveAttribute('data-rtp-frames-received');
    sample.mockResolvedValue(raw());
    toggle(true);
    await act(async () => Promise.resolve());
    expect(panel()).toHaveAttribute('data-rtp-frames-received', '100');
    expect(panel()).not.toHaveAttribute('data-rtp-receive-fps');
    clock = 2000; sample.mockResolvedValue(raw(60));
    await act(async () => vi.advanceTimersByTimeAsync(1000));
    expect(panel()).toHaveAttribute('data-rtp-receive-fps', '60');
    expect(panel()).toHaveAttribute('data-rtp-decode-fps', '60');
    expect(screen.getByLabelText('平均解码耗时（毫秒/帧）')).toHaveTextContent('10.00');
    view.rerender(<Component videoRef={videoRef} ready={false} sampleInboundVideoStats={sample} />);
    const calls = sample.mock.calls.length;
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(sample).toHaveBeenCalledTimes(calls);
    expect(panel()).not.toHaveAttribute('data-rtp-frames-received');
    view.unmount();
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(sample).toHaveBeenCalledTimes(calls);
  });
  it('does not inspect a late result after readiness loss or unmount', async () => {
    const { videoRef } = videoFixture();
    let resolve!: (value: unknown) => void;
    const sample = vi.fn(() => new Promise(done => { resolve = done; }));
    const Component = BrowserVideoStatistics as unknown as React.ComponentType<{videoRef: typeof videoRef; ready: boolean; sampleInboundVideoStats: typeof sample}>;
    const view = render(<Component videoRef={videoRef} ready sampleInboundVideoStats={sample} />);
    toggle(true);
    expect(sample).toHaveBeenCalledTimes(1);
    const read = vi.fn(() => { throw new Error('late results must not be inspected'); });
    const late = Object.defineProperty({}, 'framesReceived', { get: read });
    view.rerender(<Component videoRef={videoRef} ready={false} sampleInboundVideoStats={sample} />);
    view.unmount();
    await act(async () => resolve(late));
    expect(read).not.toHaveBeenCalled();
    await act(async () => vi.advanceTimersByTimeAsync(3000));
    expect(sample).toHaveBeenCalledTimes(1);
  });

});


describe('finite presentation cadence diagnostic', () => {
  let clock = 1000;
  let nextId = 1;
  let videoCallbacks: Map<number, VideoFrameRequestCallback>;
  let animationCallbacks: Map<number, FrameRequestCallback>;
  let requestVideo: ReturnType<typeof vi.fn>;
  let cancelVideo: ReturnType<typeof vi.fn>;
  function cadenceVideo() {
    const fixture = videoFixture();
    requestVideo = vi.fn((callback: VideoFrameRequestCallback) => { const id = nextId++; videoCallbacks.set(id, callback); return id; });
    cancelVideo = vi.fn((id: number) => { videoCallbacks.delete(id); });
    Object.defineProperties(fixture.videoRef.current, {
      requestVideoFrameCallback: { configurable: true, value: requestVideo },
      cancelVideoFrameCallback: { configurable: true, value: cancelVideo },
    });
    return fixture;
  }
  function frame(at: number, presentedFrames: number, mediaTime: number, expectedDisplayTime = at + 5, rtpTimestamp = 0) {
    clock = at;
    const first = videoCallbacks.entries().next().value as [number, VideoFrameRequestCallback] | undefined;
    if (!first) throw new Error('expected pending native video callback');
    videoCallbacks.delete(first[0]);
    act(() => first[1](at, { presentationTime: at, expectedDisplayTime, width: 1920, height: 1080,
      presentedFrames, mediaTime, rtpTimestamp }));
  }
  function animation(at: number) {
    clock = at;
    const first = animationCallbacks.entries().next().value as [number, FrameRequestCallback] | undefined;
    if (!first) throw new Error('expected pending animation callback');
    animationCallbacks.delete(first[0]);
    act(() => first[1](at));
  }
  beforeEach(() => {
    vi.useFakeTimers(); clock = 1000; nextId = 1;
    videoCallbacks = new Map(); animationCallbacks = new Map();
    vi.spyOn(performance, 'now').mockImplementation(() => clock);
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(callback => {
      const id = nextId++; animationCallbacks.set(id, callback); return id;
    });
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(id => { animationCallbacks.delete(id); });
  });
  afterEach(() => { cleanup(); vi.clearAllTimers(); vi.useRealTimers(); vi.restoreAllMocks(); });
  it('requires an explicit ready start and returns bounded numeric aggregates after 30 seconds', () => {
    const { videoRef } = cadenceVideo();
    const view = render(<BrowserVideoStatistics videoRef={videoRef} ready={false} />);
    toggle(true);
    const start = screen.getByRole('button', { name: '采样呈现节奏（30秒）' });
    expect(start).toBeDisabled();
    view.rerender(<BrowserVideoStatistics videoRef={videoRef} ready />);
    act(() => vi.advanceTimersByTime(1000));
    expect(requestVideo).not.toHaveBeenCalled();
    expect(window.requestAnimationFrame).not.toHaveBeenCalled();
    fireEvent.click(start);
    expect(start).toBeDisabled();
    expect(panel()).toHaveAttribute('data-presentation-state', 'running');
    animation(1010); animation(1026); animation(1066);
    frame(1020, 100, 2, 1025, 4294967000);
    frame(1053, 102, 2.033, 1050, 2674); // RTP wraps by 2970 ticks, metadata has two presents.
    clock = 31000;
    act(() => vi.advanceTimersByTime(30000));
    expect(panel()).toHaveAttribute('data-presentation-state', 'complete');
    expect(panel()).toHaveAttribute('data-presentation-elapsed-ms', '30000');
    expect(panel()).toHaveAttribute('data-presentation-video-callbacks', '2');
    expect(panel()).toHaveAttribute('data-presentation-presented-frame-delta', '2');
    expect(panel().getAttribute('data-presentation-media-time-delta-seconds')).not.toBeNull();
    expect(Number(panel().getAttribute('data-presentation-media-time-delta-seconds'))).toBeCloseTo(.033, 8);
    expect(panel()).toHaveAttribute('data-presentation-rtp-time-delta-ms', '33');
    expect(panel()).toHaveAttribute('data-presentation-raf-interval-p50-ms', '16');
    expect(panel()).toHaveAttribute('data-presentation-raf-interval-p95-ms', '40');
    expect(panel()).toHaveAttribute('data-presentation-raf-interval-max-ms', '40');
    expect(panel()).toHaveAttribute('data-presentation-video-callback-span-ms', '33');
    expect(panel()).toHaveAttribute('data-presentation-video-gap-p95-ms', '33');
    expect(panel()).toHaveAttribute('data-presentation-video-late-p95-ms', '3');
    expect(videoCallbacks.size).toBe(0); expect(animationCallbacks.size).toBe(0);
    expect(screen.getByText(/回调次数可能少于提交呈现的帧数/)).toBeInTheDocument();
    expect(screen.getByText(/呈现节奏不代表端到端延迟/)).toBeInTheDocument();
    const requests = requestVideo.mock.calls.length;
    act(() => vi.advanceTimersByTime(60000));
    expect(requestVideo).toHaveBeenCalledTimes(requests);
  });
  it('cancels on collapse, readiness loss or unmount and never reads late callback metadata', () => {
    const { videoRef } = cadenceVideo();
    const view = render(<BrowserVideoStatistics videoRef={videoRef} ready />);
    toggle(true); fireEvent.click(screen.getByRole('button', { name: '采样呈现节奏（30秒）' }));
    const late = videoCallbacks.values().next().value as VideoFrameRequestCallback;
    toggle(false);
    expect(cancelVideo).toHaveBeenCalled(); expect(animationCallbacks.size).toBe(0);
    const read = vi.fn(() => { throw new Error('must not inspect late metadata'); });
    const metadata = Object.defineProperty({}, 'presentedFrames', { get: read }) as VideoFrameCallbackMetadata;
    act(() => late(1100, metadata));
    expect(read).not.toHaveBeenCalled();
    toggle(true); expect(panel()).not.toHaveAttribute('data-presentation-video-callbacks');
    fireEvent.click(screen.getByRole('button', { name: '采样呈现节奏（30秒）' }));
    view.rerender(<BrowserVideoStatistics videoRef={videoRef} ready={false} />);
    expect(panel()).toHaveAttribute('data-presentation-state', 'cancelled');
    expect(videoCallbacks.size).toBe(0); expect(animationCallbacks.size).toBe(0);
    view.rerender(<BrowserVideoStatistics videoRef={videoRef} ready />);
    fireEvent.click(screen.getByRole('button', { name: '采样呈现节奏（30秒）' }));
    view.unmount();
    expect(videoCallbacks.size).toBe(0); expect(animationCallbacks.size).toBe(0);
  });
  it('marks unsupported native callbacks unavailable and does not create a fallback loop', () => {
    const { videoRef } = videoFixture();
    render(<BrowserVideoStatistics videoRef={videoRef} ready />); toggle(true);
    expect(screen.getByRole('button', { name: '采样呈现节奏（30秒）' })).toBeDisabled();
    expect(panel()).toHaveAttribute('data-presentation-state', 'unavailable');
    expect(window.requestAnimationFrame).not.toHaveBeenCalled();
  });
  it('keeps invalid or rolled-back numeric metadata unknown without leaking extra fields', () => {
    const { videoRef } = cadenceVideo();
    render(<BrowserVideoStatistics videoRef={videoRef} ready />); toggle(true);
    fireEvent.click(screen.getByRole('button', { name: '采样呈现节奏（30秒）' }));
    frame(1020, 100, 2, 1010, 20000);
    frame(1040, 98, 1, Number.NaN, 19000);
    frame(1060, 110, 3, Number.POSITIVE_INFINITY, 21000);
    clock = 31000; act(() => vi.advanceTimersByTime(30000));
    expect(panel()).toHaveAttribute('data-presentation-state', 'complete');
    expect(panel()).not.toHaveAttribute('data-presentation-presented-frame-delta');
    expect(panel()).not.toHaveAttribute('data-presentation-media-time-delta-seconds');
    expect(panel()).not.toHaveAttribute('data-presentation-rtp-time-delta-ms');
    expect(panel().outerHTML).not.toMatch(/NaN|Infinity|data-[\w-]+="-\d/);
  });
  it('cancels when the page becomes hidden and does not retain background callbacks', () => {
    const { videoRef } = cadenceVideo();
    render(<BrowserVideoStatistics videoRef={videoRef} ready />); toggle(true);
    fireEvent.click(screen.getByRole('button', { name: '采样呈现节奏（30秒）' }));
    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
    act(() => document.dispatchEvent(new Event('visibilitychange')));
    expect(panel()).toHaveAttribute('data-presentation-state', 'cancelled');
    expect(videoCallbacks.size).toBe(0); expect(animationCallbacks.size).toBe(0);
    visibility.mockRestore();
  });
  it('bounds each callback category to 8192 and never reads unrelated metadata properties', () => {
    const { videoRef } = cadenceVideo();
    render(<BrowserVideoStatistics videoRef={videoRef} ready />); toggle(true);
    fireEvent.click(screen.getByRole('button', { name: '采样呈现节奏（30秒）' }));
    const read = vi.fn(() => { throw new Error('unrelated metadata must not be inspected'); });
    const metadata = Object.defineProperties({ presentedFrames: 1, mediaTime: 1, expectedDisplayTime: 1010, rtpTimestamp: 0 }, {
      captureTime: { get: read }, processingDuration: { get: read }, width: { get: read }, height: { get: read },
    }) as VideoFrameCallbackMetadata;
    clock = 1010;
    act(() => { const pending = videoCallbacks.entries().next().value as [number, VideoFrameRequestCallback]; videoCallbacks.delete(pending[0]); pending[1](1010, metadata); });
    expect(read).not.toHaveBeenCalled();
    act(() => {
      for (let n = 0; n < 8192; n++) {
        clock = 1020 + n;
        const pending = animationCallbacks.entries().next().value as [number, FrameRequestCallback];
        animationCallbacks.delete(pending[0]); pending[1](clock);
      }
    });
    expect(panel()).toHaveAttribute('data-presentation-state', 'complete');
    expect(panel()).toHaveAttribute('data-presentation-animation-callbacks', '8192');
    expect(panel()).toHaveAttribute('data-presentation-sample-limit-reached', '1');
    expect(videoCallbacks.size).toBe(0); expect(animationCallbacks.size).toBe(0);
  });
  it('cancels when the video element changes and bounds delayed-timer callbacks at the deadline', () => {
    const { videoRef } = cadenceVideo();
    const view = render(<BrowserVideoStatistics videoRef={videoRef} ready />); toggle(true);
    fireEvent.click(screen.getByRole('button', { name: '采样呈现节奏（30秒）' }));
    const oldCallbacks = [...videoCallbacks.values()];
    const replacement = cadenceVideo();
    view.rerender(<BrowserVideoStatistics videoRef={replacement.videoRef} ready />);
    expect(videoCallbacks.size).toBe(0); expect(animationCallbacks.size).toBe(0);
    fireEvent.click(screen.getByRole('button', { name: '采样呈现节奏（30秒）' }));
    clock = 32000;
    const read = vi.fn(() => { throw new Error('expired sample cannot inspect metadata'); });
    const expired = Object.defineProperty({}, 'presentedFrames', { get: read }) as VideoFrameCallbackMetadata;
    act(() => { const pending = videoCallbacks.values().next().value as VideoFrameRequestCallback; pending(32000, expired); oldCallbacks[0]!(32000, expired); });
    expect(read).not.toHaveBeenCalled();
    expect(panel()).toHaveAttribute('data-presentation-state', 'complete');
    expect(panel()).toHaveAttribute('data-presentation-video-callbacks', '0');
    expect(videoCallbacks.size).toBe(0); expect(animationCallbacks.size).toBe(0);
  });
});
