import { useEffect, useState, type RefObject } from 'react';

type BrowserVideoStatisticsProps = {
  videoRef: RefObject<HTMLVideoElement | null>;
  ready: boolean;
};

type Counters = { total: number; dropped: number; corrupted: number };
type Delta = Counters & { elapsedMs: number; perSecond: number };
type Sample = {
  sampledAtMs: number | null;
  width: number | null;
  height: number | null;
  currentTime: number | null;
  readyState: number | null;
  paused: boolean;
  ended: boolean;
  qualityStatus: 'available' | 'unsupported' | 'unavailable' | 'invalid';
  counters: Counters | null;
  delta: Delta | null;
};

const count = (value: number) => Number.isSafeInteger(value) && value >= 0 ? value : null;
const nonnegative = (value: number) => Number.isFinite(value) && value >= 0 ? value : null;
const display = (value: number | null | undefined, decimals?: number) => value == null ? '不可用' : decimals === undefined ? String(value) : value.toFixed(decimals);

export function BrowserVideoStatistics({ videoRef, ready }: BrowserVideoStatisticsProps) {
  const [expanded, setExpanded] = useState(false);
  const [sample, setSample] = useState<Sample | null>(null);

  useEffect(() => {
    setSample(null);
    if (!expanded || !ready) return;
    let previous: { video: HTMLVideoElement; sampledAtMs: number; counters: Counters } | null = null;
    const sampleVideo = () => {
      const video = videoRef.current;
      if (!video) { previous = null; setSample(null); return; }
      const sampledAtMs = nonnegative(performance.now());
      let qualityStatus: Sample['qualityStatus'] = 'unsupported';
      let counters: Counters | null = null;
      if (typeof video.getVideoPlaybackQuality === 'function') {
        try {
          const quality = video.getVideoPlaybackQuality();
          const total = count(quality.totalVideoFrames);
          const dropped = count(quality.droppedVideoFrames);
          const corrupted = count(quality.corruptedVideoFrames);
          if (total !== null && dropped !== null && corrupted !== null && dropped <= total && corrupted <= total) {
            counters = { total, dropped, corrupted };
            qualityStatus = 'available';
          } else qualityStatus = 'invalid';
        } catch { qualityStatus = 'unavailable'; }
      }
      let delta: Delta | null = null;
      if (previous && previous.video === video && counters && sampledAtMs !== null) {
        const elapsedMs = sampledAtMs - previous.sampledAtMs;
        const total = counters.total - previous.counters.total;
        const dropped = counters.dropped - previous.counters.dropped;
        const corrupted = counters.corrupted - previous.counters.corrupted;
        const perSecond = total * 1000 / elapsedMs;
        if (elapsedMs > 0 && Number.isFinite(elapsedMs) && total >= 0 && dropped >= 0 && corrupted >= 0 && Number.isFinite(perSecond)) {
          delta = { elapsedMs, total, dropped, corrupted, perSecond };
        }
      }
      previous = counters && sampledAtMs !== null ? { video, sampledAtMs, counters } : null;
      const readyState = count(video.readyState);
      setSample({
        sampledAtMs, width: count(video.videoWidth), height: count(video.videoHeight),
        currentTime: nonnegative(video.currentTime), readyState: readyState !== null && readyState <= 4 ? readyState : null,
        paused: video.paused, ended: video.ended, qualityStatus, counters, delta,
      });
    };
    sampleVideo();
    const timer = window.setInterval(sampleVideo, 1000);
    return () => window.clearInterval(timer);
  }, [expanded, ready, videoRef]);

  const fields = sample ? [
    ['分辨率', sample.width === null || sample.height === null ? '不可用' : `${sample.width} × ${sample.height}`],
    ['播放时间（秒）', display(sample.currentTime, 3)],
    ['readyState', display(sample.readyState)],
    ['已暂停', sample.paused ? '是' : '否'], ['已结束', sample.ended ? '是' : '否'],
    ['总帧数', display(sample.counters?.total)], ['丢帧数', display(sample.counters?.dropped)],
    ['损坏帧数', display(sample.counters?.corrupted)],
    ['采样时刻（单调毫秒）', display(sample.sampledAtMs, 2)],
    ['采样间隔（毫秒）', display(sample.delta?.elapsedMs, 2)],
    ['总帧增量', display(sample.delta?.total)], ['丢帧增量', display(sample.delta?.dropped)],
    ['损坏帧增量', display(sample.delta?.corrupted)], ['帧计数/秒', display(sample.delta?.perSecond, 2)],
  ] : [];

  return <details open={expanded} onToggle={event => setExpanded(event.currentTarget.open)}
    className="rounded-lg border border-white/10 px-3 py-2 text-xs text-slate-400"
    data-browser-video-statistics="true" data-state={expanded ? ready ? 'sampling' : 'waiting' : 'collapsed'}
    data-quality-status={sample?.qualityStatus} data-sampled-at-ms={sample?.sampledAtMs ?? undefined}
    data-video-width={sample?.width ?? undefined} data-video-height={sample?.height ?? undefined}
    data-current-time={sample?.currentTime ?? undefined} data-ready-state={sample?.readyState ?? undefined}
    data-paused={sample?.paused} data-ended={sample?.ended}
    data-total-video-frames={sample?.counters?.total} data-dropped-video-frames={sample?.counters?.dropped}
    data-corrupted-video-frames={sample?.counters?.corrupted} data-elapsed-ms={sample?.delta?.elapsedMs}
    data-total-frame-delta={sample?.delta?.total} data-dropped-frame-delta={sample?.delta?.dropped}
    data-corrupted-frame-delta={sample?.delta?.corrupted} data-frame-count-per-second={sample?.delta?.perSecond}>
    <summary className="cursor-pointer rounded focus-visible:outline-2 focus-visible:outline-cyan-400">播放统计</summary>
    {expanded && <div className="mt-3 space-y-3">
      {!sample && <p>画面就绪后每秒采样。</p>}
      {sample?.qualityStatus === 'unsupported' && <p>当前浏览器不支持播放帧计数，计数和帧率不可用。</p>}
      {(sample?.qualityStatus === 'invalid' || sample?.qualityStatus === 'unavailable') && <p>播放帧计数不可用。</p>}
      {sample && <dl className="grid grid-cols-2 gap-x-5 gap-y-2 sm:grid-cols-3 lg:grid-cols-4">
        {fields.map(([label, value]) => <div key={label}><dt>{label}</dt><dd className="mt-1 font-mono text-slate-200"><output aria-label={label}>{value}</output></dd></div>)}
      </dl>}
      <p>这些是浏览器播放计数，不代表独立源画面、RTP 帧数或网络延迟。帧计数/秒需要两个有效连续样本。</p>
    </div>}
  </details>;
}
