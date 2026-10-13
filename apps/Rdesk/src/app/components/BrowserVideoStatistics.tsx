import { useEffect, useRef, useState, type RefObject } from 'react';
import { usePresentationCadenceDiagnostics } from '../hooks/usePresentationCadenceDiagnostics';
import { boundedInboundVideoStatistics, inboundVideoStatisticKeys, type BrowserInboundVideoStatistics } from '../services/browserRemoteSessionService';

type BrowserVideoStatisticsProps = {
  videoRef: RefObject<HTMLVideoElement | null>;
  ready: boolean;
  sampleInboundVideoStats?: () => Promise<BrowserInboundVideoStatistics | null>;
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


type InboundSample = { sampledAtMs: number | null; counters: BrowserInboundVideoStatistics };
type InboundRates = { receiveFps: number | null; decodeFps: number | null; decodeMs: number | null; jitterBufferMs: number | null; receiveMbps: number | null };
const safeDiagnosticNumber = (value: number) => Number.isFinite(value) && value >= 0 && value <= Number.MAX_SAFE_INTEGER ? value : null;
export function inboundVideoRates(previous: InboundSample | null, current: InboundSample): InboundRates | null {
  if (!previous || previous.sampledAtMs === null || current.sampledAtMs === null) return null;
  const elapsedMs = current.sampledAtMs - previous.sampledAtMs;
  if (safeDiagnosticNumber(elapsedMs) === null || elapsedMs <= 0) return null;
  // Jitter is an instantaneous gauge; the other fields are cumulative counters.
  for (const key of inboundVideoStatisticKeys) {
    if (key === 'jitter') continue;
    const before = previous.counters[key], after = current.counters[key];
    if (before !== null && after !== null && after < before) return null;
  }
  const delta = (key: keyof BrowserInboundVideoStatistics) => {
    const before = previous.counters[key], after = current.counters[key];
    return before === null || after === null ? null : safeDiagnosticNumber(after - before);
  };
  const ratio = (numerator: number | null, denominator: number | null, scale: number) =>
    numerator === null || denominator === null || denominator <= 0 ? null : safeDiagnosticNumber(numerator / denominator * scale);
  return {
    receiveFps: ratio(delta('framesReceived'), elapsedMs, 1000),
    decodeFps: ratio(delta('framesDecoded'), elapsedMs, 1000),
    decodeMs: ratio(delta('totalDecodeTime'), delta('framesDecoded'), 1000),
    jitterBufferMs: ratio(delta('jitterBufferDelay'), delta('jitterBufferEmittedCount'), 1000),
    receiveMbps: ratio(delta('bytesReceived'), elapsedMs, 0.008),
  };
}

export function BrowserVideoStatistics({ videoRef, ready, sampleInboundVideoStats }: BrowserVideoStatisticsProps) {
  const [expanded, setExpanded] = useState(false);
  const [sample, setSample] = useState<Sample | null>(null);
  const [inbound, setInbound] = useState<{ sample: InboundSample; rates: InboundRates | null } | null>(null);
  const inboundPending = useRef(false);
  const cadence = usePresentationCadenceDiagnostics(videoRef, expanded && ready);
  const cadenceResult = expanded && ready ? cadence.summary : null;

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


  useEffect(() => {
    setInbound(null);
    if (!expanded || !ready || !sampleInboundVideoStats) return;
    let active = true;
    let previous: InboundSample | null = null;
    const poll = async () => {
      if (!active || inboundPending.current) return;
      inboundPending.current = true;
      try {
        const raw = await sampleInboundVideoStats();
        if (!active) return;
        if (raw === null) { previous = null; setInbound(null); return; }
        const current = { sampledAtMs: safeDiagnosticNumber(performance.now()), counters: boundedInboundVideoStatistics(raw) };
        const rates = inboundVideoRates(previous, current);
        previous = current.sampledAtMs === null ? null : current;
        setInbound({ sample: current, rates });
      } catch {
        if (active) { previous = null; setInbound(null); }
      } finally { inboundPending.current = false; }
    };
    void poll();
    const timer = window.setInterval(() => { void poll(); }, 1000);
    return () => { active = false; previous = null; window.clearInterval(timer); };
  }, [expanded, ready, sampleInboundVideoStats]);
  const visibleInbound = expanded && ready ? inbound : null;
  const inboundFields = [
    ['RTP 接收帧数', display(visibleInbound?.sample.counters.framesReceived)],
    ['RTP 解码帧数', display(visibleInbound?.sample.counters.framesDecoded)],
    ['RTP 丢弃帧数', display(visibleInbound?.sample.counters.framesDropped)],
    ['RTP 丢包数', display(visibleInbound?.sample.counters.packetsLost)],
    ['累计解码时间（秒）', display(visibleInbound?.sample.counters.totalDecodeTime, 6)],
    ['接收字节数', display(visibleInbound?.sample.counters.bytesReceived)],
    ['当前抖动（秒）', display(visibleInbound?.sample.counters.jitter, 6)],
    ['累计抖动缓冲时间（秒）', display(visibleInbound?.sample.counters.jitterBufferDelay, 6)],
    ['抖动缓冲输出计数', display(visibleInbound?.sample.counters.jitterBufferEmittedCount)],
    ['RTP 接收帧/秒', display(visibleInbound?.rates?.receiveFps, 2)],
    ['RTP 解码帧/秒', display(visibleInbound?.rates?.decodeFps, 2)],
    ['平均解码耗时（毫秒/帧）', display(visibleInbound?.rates?.decodeMs, 2)],
    ['平均抖动缓冲停留（毫秒）', display(visibleInbound?.rates?.jitterBufferMs, 2)],
    ['接收速率（Mbps）', display(visibleInbound?.rates?.receiveMbps, 3)],
  ];

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
    data-presentation-state={cadence.state}
    data-presentation-elapsed-ms={cadenceResult?.elapsedMs ?? undefined}
    data-presentation-video-callbacks={cadenceResult?.videoCallbacks ?? undefined}
    data-presentation-animation-callbacks={cadenceResult?.animationCallbacks ?? undefined}
    data-presentation-presented-frame-delta={cadenceResult?.presentedFrameDelta ?? undefined}
    data-presentation-media-time-delta-seconds={cadenceResult?.mediaTimeDeltaSeconds ?? undefined}
    data-presentation-rtp-time-delta-ms={cadenceResult?.rtpTimeDeltaMs ?? undefined}
    data-presentation-raf-interval-p50-ms={cadenceResult?.rafIntervalP50Ms ?? undefined}
    data-presentation-raf-interval-p95-ms={cadenceResult?.rafIntervalP95Ms ?? undefined}
    data-presentation-raf-interval-max-ms={cadenceResult?.rafIntervalMaxMs ?? undefined}
    data-presentation-video-callback-span-ms={cadenceResult?.videoCallbackSpanMs ?? undefined}
    data-presentation-video-gap-p95-ms={cadenceResult?.videoGapP95Ms ?? undefined}
    data-presentation-video-late-p95-ms={cadenceResult?.videoLateP95Ms ?? undefined}
    data-presentation-sample-limit-reached={cadenceResult?.sampleLimitReached ?? undefined}
    data-browser-video-statistics="true" data-state={expanded ? ready ? 'sampling' : 'waiting' : 'collapsed'}
    data-rtp-sampled-at-ms={visibleInbound?.sample.sampledAtMs ?? undefined}
    data-rtp-frames-received={visibleInbound?.sample.counters.framesReceived ?? undefined}
    data-rtp-frames-decoded={visibleInbound?.sample.counters.framesDecoded ?? undefined}
    data-rtp-frames-dropped={visibleInbound?.sample.counters.framesDropped ?? undefined}
    data-rtp-packets-lost={visibleInbound?.sample.counters.packetsLost ?? undefined}
    data-rtp-total-decode-time={visibleInbound?.sample.counters.totalDecodeTime ?? undefined}
    data-rtp-bytes-received={visibleInbound?.sample.counters.bytesReceived ?? undefined}
    data-rtp-jitter={visibleInbound?.sample.counters.jitter ?? undefined}
    data-rtp-jitter-buffer-delay={visibleInbound?.sample.counters.jitterBufferDelay ?? undefined}
    data-rtp-jitter-buffer-emitted-count={visibleInbound?.sample.counters.jitterBufferEmittedCount ?? undefined}
    data-rtp-receive-fps={visibleInbound?.rates?.receiveFps ?? undefined}
    data-rtp-decode-fps={visibleInbound?.rates?.decodeFps ?? undefined}
    data-rtp-decode-ms={visibleInbound?.rates?.decodeMs ?? undefined}
    data-rtp-jitter-buffer-ms={visibleInbound?.rates?.jitterBufferMs ?? undefined}
    data-rtp-receive-mbps={visibleInbound?.rates?.receiveMbps ?? undefined}
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
      <div className="space-y-2">
        <button type="button" onClick={cadence.start} disabled={!cadence.canStart}
          className="rounded border border-white/20 px-3 py-1 text-slate-200 disabled:opacity-50">采样呈现节奏（30秒）</button>
        <p>{cadence.state === 'running' ? '正在采样；30 秒后自动停止，收起统计或断开连接会取消。' : cadence.state === 'cancelled' ? '采样已取消，可重新开始。' : cadence.state === 'unavailable' ? '当前状态或浏览器不支持呈现节奏采样。' : '点击后进行一次有限采样。'}</p>
        {cadenceResult && <dl className="grid grid-cols-2 gap-x-5 gap-y-2 sm:grid-cols-3 lg:grid-cols-4">
          {[
            ['呈现采样时长（毫秒）', display(cadenceResult.elapsedMs, 2)],
            ['视频回调次数', display(cadenceResult.videoCallbacks)],
            ['动画回调次数', display(cadenceResult.animationCallbacks)],
            ['提交呈现帧数增量', display(cadenceResult.presentedFrameDelta)],
            ['视频媒体时间跨度（秒）', display(cadenceResult.mediaTimeDeltaSeconds, 6)],
            ['呈现帧 RTP 时间跨度（毫秒）', display(cadenceResult.rtpTimeDeltaMs, 2)],
            ['动画回调间隔 p50（毫秒）', display(cadenceResult.rafIntervalP50Ms, 2)],
            ['动画回调间隔 p95（毫秒）', display(cadenceResult.rafIntervalP95Ms, 2)],
            ['动画回调最大间隔（毫秒）', display(cadenceResult.rafIntervalMaxMs, 2)],
            ['首末视频回调跨度（毫秒）', display(cadenceResult.videoCallbackSpanMs, 2)],
            ['视频回调间隔 p95（毫秒）', display(cadenceResult.videoGapP95Ms, 2)],
            ['视频回调晚于预期显示 p95（毫秒）', display(cadenceResult.videoLateP95Ms, 2)],
            ['达到采样数量上限', cadenceResult.sampleLimitReached ? '是' : '否'],
          ].map(([label, value]) => <div key={label}><dt>{label}</dt><dd className="mt-1 font-mono text-slate-200"><output aria-label={label}>{value}</output></dd></div>)}
        </dl>}
        <p>回调次数可能少于提交呈现的帧数；帧数和媒体时间跨度从首末有效回调计算。呈现节奏不代表端到端延迟，也不测量独立源画面数量。浏览器繁忙时停止计时器可能晚于 30 秒；每类至多采集 8192 次回调。</p>
      </div>
      {!sample && <p>画面就绪后每秒采样。</p>}
      {sample?.qualityStatus === 'unsupported' && <p>当前浏览器不支持播放帧计数，计数和帧率不可用。</p>}
      {(sample?.qualityStatus === 'invalid' || sample?.qualityStatus === 'unavailable') && <p>播放帧计数不可用。</p>}
      {sample && <dl className="grid grid-cols-2 gap-x-5 gap-y-2 sm:grid-cols-3 lg:grid-cols-4">
        {fields.map(([label, value]) => <div key={label}><dt>{label}</dt><dd className="mt-1 font-mono text-slate-200"><output aria-label={label}>{value}</output></dd></div>)}
      </dl>}
      <div><p className="mb-2">视频接收与解码统计</p><dl className="grid grid-cols-2 gap-x-5 gap-y-2 sm:grid-cols-3 lg:grid-cols-4">
        {inboundFields.map(([label, value]) => <div key={label}><dt>{label}</dt><dd className="mt-1 font-mono text-slate-200"><output aria-label={label}>{value}</output></dd></div>)}
      </dl><p className="mt-2">接收、解码和播放使用各自计数。解码耗时与抖动缓冲停留按连续样本计算，不代表端到端延迟。</p></div>
      <p>这些是浏览器播放计数，不代表独立源画面、RTP 帧数或网络延迟。帧计数/秒需要两个有效连续样本。</p>
    </div>}
  </details>;
}
