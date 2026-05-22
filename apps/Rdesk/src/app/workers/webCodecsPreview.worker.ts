/// <reference lib="webworker" />

type WebCodecsFrameHeader = {
  type: "mrd.webcodecs.frame.v1";
  sequence: number;
  timestamp_us: number;
  duration_us: number;
  capture_unix_us: number;
  keyframe: boolean;
  codec: string;
  codec_format: "annexb";
  width: number;
  height: number;
};

type WebCodecsReadyMessage = {
  type: "mrd.webcodecs.ready.v1";
  session_id: string;
  codec: string;
  codec_format: "annexb";
  width: number;
  height: number;
  fps: number;
  bitrate_mbps: number;
};

type WebCodecsErrorMessage = {
  type?: string;
  code?: string;
  message?: string;
};

type WebCodecsAccessUnitMessage = {
  header: WebCodecsFrameHeader;
  payload: Uint8Array;
};

type StartMessage = {
  type: "start";
  canvas: OffscreenCanvas;
  websocketUrl: string;
  sessionId: string;
  fps: number;
  width: number;
  height: number;
  bitrateMbps: number;
  h264Profile: string;
  viewportWidth: number;
  viewportHeight: number;
  devicePixelRatio: number;
};

type ResizeMessage = {
  type: "resize";
  viewportWidth: number;
  viewportHeight: number;
  devicePixelRatio: number;
};

type StopMessage = {
  type: "stop";
};

type WorkerMessage = StartMessage | ResizeMessage | StopMessage;

type WorkerToMainMessage =
  | {
      type: "ready";
      width: number;
      height: number;
      fps: number;
      bitrateMbps: number;
    }
  | {
      type: "stats";
      fps: number;
      paintFps: number;
      frameCount: number;
      frameIntervalP95Ms: number | null;
      latencyLatestMs: number;
      latencyP50Ms: number;
      latencyP95Ms: number;
      latencyMaxMs: number;
      latencySamples: number;
      decodeQueueSize: number;
      droppedFrames: number;
      canvasWidth: number;
      canvasHeight: number;
    }
  | {
      type: "closed";
    }
  | {
      type: "error";
      message: string;
    };

const WEBCODECS_CHUNK_MAGIC = "MRDWC01\0";
const WEBCODECS_BINARY_HEADER_LEN = 12;
const MAX_TRACKED_SAMPLES = 240;

let socket: WebSocket | null = null;
let decoder: VideoDecoder | null = null;
let canvas: OffscreenCanvas | null = null;
let context: OffscreenCanvasRenderingContext2D | null = null;
let configured = false;
let viewportWidth = 2;
let viewportHeight = 2;
let viewportDevicePixelRatio = 1;
let lastOutputAt: number | null = null;
let lastStatsAt = performance.now();
let framesSinceStats = 0;
let totalFrames = 0;
let droppedFrames = 0;
const frameIntervalsMs: number[] = [];
const latencySamplesMs: number[] = [];
const headersByTimestamp = new Map<number, WebCodecsFrameHeader>();

function post(message: WorkerToMainMessage) {
  self.postMessage(message);
}

function percentile(values: number[], percentileValue: number) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * percentileValue) - 1));
  return sorted[index] ?? null;
}

function parseAccessUnit(data: ArrayBuffer): WebCodecsAccessUnitMessage | null {
  if (data.byteLength < WEBCODECS_BINARY_HEADER_LEN) return null;
  const view = new DataView(data);
  let magic = "";
  for (let index = 0; index < 8; index += 1) {
    magic += String.fromCharCode(view.getUint8(index));
  }
  if (magic !== WEBCODECS_CHUNK_MAGIC) return null;
  const headerLength = view.getUint32(8, true);
  const payloadOffset = WEBCODECS_BINARY_HEADER_LEN + headerLength;
  if (payloadOffset > data.byteLength) return null;
  try {
    const headerBytes = new Uint8Array(data, WEBCODECS_BINARY_HEADER_LEN, headerLength);
    const header = JSON.parse(new TextDecoder().decode(headerBytes)) as WebCodecsFrameHeader;
    if (header.type !== "mrd.webcodecs.frame.v1") return null;
    return {
      header,
      payload: new Uint8Array(data, payloadOffset),
    };
  } catch {
    return null;
  }
}

function applyViewport(nextWidth: number, nextHeight: number, nextScale: number) {
  viewportWidth = Math.max(2, Math.round(nextWidth || 2));
  viewportHeight = Math.max(2, Math.round(nextHeight || 2));
  viewportDevicePixelRatio = Math.max(1, nextScale || 1);
}

function resizeCanvasForFrame(frame: VideoFrame) {
  if (!canvas) return;
  const displayWidth = frame.displayWidth || frame.codedWidth;
  const displayHeight = frame.displayHeight || frame.codedHeight;
  const maxCanvasWidth = Math.max(2, Math.round(viewportWidth * viewportDevicePixelRatio));
  const maxCanvasHeight = Math.max(2, Math.round(viewportHeight * viewportDevicePixelRatio));
  const displayAspect = displayWidth / Math.max(1, displayHeight);
  let targetWidth = Math.min(displayWidth, maxCanvasWidth);
  let targetHeight = Math.round(targetWidth / displayAspect);
  if (targetHeight > Math.min(displayHeight, maxCanvasHeight)) {
    targetHeight = Math.min(displayHeight, maxCanvasHeight);
    targetWidth = Math.round(targetHeight * displayAspect);
  }
  targetWidth = Math.max(2, targetWidth);
  targetHeight = Math.max(2, targetHeight);
  if (canvas.width !== targetWidth || canvas.height !== targetHeight) {
    canvas.width = targetWidth;
    canvas.height = targetHeight;
  }
}

function recordLatency(frame: VideoFrame) {
  const header = headersByTimestamp.get(frame.timestamp);
  if (!header) return null;
  headersByTimestamp.delete(frame.timestamp);
  const latestMs = performance.timeOrigin + performance.now() - header.capture_unix_us / 1000;
  latencySamplesMs.push(latestMs);
  if (latencySamplesMs.length > MAX_TRACKED_SAMPLES) latencySamplesMs.shift();
  return latestMs;
}

function handleDecodedFrame(frame: VideoFrame) {
  try {
    const now = performance.now();
    const latestLatencyMs = recordLatency(frame);
    if (lastOutputAt !== null) {
      frameIntervalsMs.push(now - lastOutputAt);
      if (frameIntervalsMs.length > MAX_TRACKED_SAMPLES) frameIntervalsMs.shift();
    }
    lastOutputAt = now;
    framesSinceStats += 1;
    totalFrames += 1;
    resizeCanvasForFrame(frame);
    context?.drawImage(frame, 0, 0, canvas?.width ?? 2, canvas?.height ?? 2);

    const elapsedMs = now - lastStatsAt;
    if (elapsedMs >= 500 && latestLatencyMs !== null) {
      const fps = (framesSinceStats * 1000) / elapsedMs;
      post({
        type: "stats",
        fps,
        paintFps: fps,
        frameCount: totalFrames,
        frameIntervalP95Ms: percentile(frameIntervalsMs, 0.95),
        latencyLatestMs: latestLatencyMs,
        latencyP50Ms: percentile(latencySamplesMs, 0.5) ?? latestLatencyMs,
        latencyP95Ms: percentile(latencySamplesMs, 0.95) ?? latestLatencyMs,
        latencyMaxMs: Math.max(...latencySamplesMs),
        latencySamples: latencySamplesMs.length,
        decodeQueueSize: decoder?.decodeQueueSize ?? 0,
        droppedFrames,
        canvasWidth: canvas?.width ?? 0,
        canvasHeight: canvas?.height ?? 0,
      });
      framesSinceStats = 0;
      lastStatsAt = now;
    }
  } finally {
    frame.close();
  }
}

async function configureDecoder(ready: WebCodecsReadyMessage) {
  if (configured || !decoder) return;
  const config = {
    codec: ready.codec,
    codedWidth: ready.width,
    codedHeight: ready.height,
    hardwareAcceleration: "prefer-software",
    optimizeForLatency: true,
    avc: { format: "annexb" },
  } as VideoDecoderConfig & { avc: { format: "annexb" } };
  const support = await VideoDecoder.isConfigSupported(config).catch(() => null);
  if (support && !support.supported) {
    throw new Error(`WebCodecs worker decoder does not support ${ready.codec} annexb`);
  }
  decoder.configure(support?.config ?? config);
  configured = true;
  post({
    type: "ready",
    width: ready.width,
    height: ready.height,
    fps: ready.fps,
    bitrateMbps: ready.bitrate_mbps,
  });
}

function stop() {
  configured = false;
  try {
    socket?.send(JSON.stringify({ type: "stop" }));
  } catch {
    // Best-effort cleanup.
  }
  socket?.close();
  socket = null;
  decoder?.close();
  decoder = null;
  post({ type: "closed" });
}

function start(message: StartMessage) {
  stop();
  if (!("VideoDecoder" in self) || !("EncodedVideoChunk" in self)) {
    post({ type: "error", message: "WebCodecs worker VideoDecoder / EncodedVideoChunk is unavailable" });
    return;
  }

  canvas = message.canvas;
  context = canvas.getContext("2d", { alpha: false });
  if (!context) {
    post({ type: "error", message: "WebCodecs worker failed to create OffscreenCanvas 2D context" });
    return;
  }
  applyViewport(message.viewportWidth, message.viewportHeight, message.devicePixelRatio);
  lastOutputAt = null;
  lastStatsAt = performance.now();
  framesSinceStats = 0;
  totalFrames = 0;
  droppedFrames = 0;
  frameIntervalsMs.length = 0;
  latencySamplesMs.length = 0;
  headersByTimestamp.clear();

  decoder = new VideoDecoder({
    output: handleDecodedFrame,
    error: (error) => {
      post({ type: "error", message: `WebCodecs worker decode failed: ${error.message}` });
    },
  });

  socket = new WebSocket(message.websocketUrl);
  socket.binaryType = "arraybuffer";
  socket.onopen = () => {
    socket?.send(
      JSON.stringify({
        type: "start",
        session_id: message.sessionId,
        fps: message.fps,
        width: message.width,
        height: message.height,
        bitrate_mbps: message.bitrateMbps,
        h264_profile: message.h264Profile,
      })
    );
  };
  socket.onmessage = (event) => {
    void (async () => {
      if (typeof event.data === "string") {
        const payload = JSON.parse(event.data) as WebCodecsReadyMessage | WebCodecsErrorMessage;
        if (payload.type === "mrd.webcodecs.ready.v1") {
          await configureDecoder(payload as WebCodecsReadyMessage);
        } else if ("message" in payload && payload.message) {
          post({ type: "error", message: payload.message });
        }
        return;
      }
      const buffer =
        event.data instanceof ArrayBuffer ? event.data : await (event.data as Blob).arrayBuffer();
      const accessUnit = parseAccessUnit(buffer);
      if (!accessUnit || !configured || !decoder) return;
      if (decoder.decodeQueueSize > 2 && !accessUnit.header.keyframe) {
        droppedFrames += 1;
        return;
      }
      headersByTimestamp.set(accessUnit.header.timestamp_us, accessUnit.header);
      const chunkData = accessUnit.payload.buffer.slice(
        accessUnit.payload.byteOffset,
        accessUnit.payload.byteOffset + accessUnit.payload.byteLength
      ) as ArrayBuffer;
      decoder.decode(
        new EncodedVideoChunk({
          type: accessUnit.header.keyframe ? "key" : "delta",
          timestamp: accessUnit.header.timestamp_us,
          duration: accessUnit.header.duration_us,
          data: chunkData,
        })
      );
    })().catch((error) => {
      post({ type: "error", message: error instanceof Error ? error.message : String(error) });
    });
  };
  socket.onerror = () => {
    post({ type: "error", message: "WebCodecs worker WebSocket failed" });
  };
  socket.onclose = () => {
    post({ type: "closed" });
  };
}

self.onmessage = (event: MessageEvent<WorkerMessage>) => {
  const message = event.data;
  if (message.type === "start") {
    start(message);
    return;
  }
  if (message.type === "resize") {
    applyViewport(message.viewportWidth, message.viewportHeight, message.devicePixelRatio);
    return;
  }
  stop();
};
