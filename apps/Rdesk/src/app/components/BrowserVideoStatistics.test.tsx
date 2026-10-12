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
