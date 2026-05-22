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
      rendererBackend: WebCodecsWorkerRenderBackend;
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
      rendererBackend: WebCodecsWorkerRenderBackend;
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
const STALE_DELTA_FRAME_DROP_MS = 28;
const KEYFRAME_REQUEST_COOLDOWN_MS = 250;

type WebCodecsWorkerRenderBackend = "webgl2" | "2d";

type FrameRenderer = {
  backend: WebCodecsWorkerRenderBackend;
  draw: (frame: VideoFrame) => void;
};

let socket: WebSocket | null = null;
let decoder: VideoDecoder | null = null;
let canvas: OffscreenCanvas | null = null;
let renderer: FrameRenderer | null = null;
let configured = false;
let viewportWidth = 2;
let viewportHeight = 2;
let viewportDevicePixelRatio = 1;
let lastOutputAt: number | null = null;
let lastStatsAt = performance.now();
let framesSinceStats = 0;
let totalFrames = 0;
let droppedFrames = 0;
let lastKeyframeRequestAt = 0;
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

function compileShader(gl: WebGL2RenderingContext, type: number, source: string) {
  const shader = gl.createShader(type);
  if (!shader) throw new Error("createShader failed");
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(shader) ?? "unknown shader error";
    gl.deleteShader(shader);
    throw new Error(log);
  }
  return shader;
}

function linkProgram(gl: WebGL2RenderingContext, vertexSource: string, fragmentSource: string) {
  const vertexShader = compileShader(gl, gl.VERTEX_SHADER, vertexSource);
  const fragmentShader = compileShader(gl, gl.FRAGMENT_SHADER, fragmentSource);
  const program = gl.createProgram();
  if (!program) throw new Error("createProgram failed");
  gl.attachShader(program, vertexShader);
  gl.attachShader(program, fragmentShader);
  gl.linkProgram(program);
  gl.deleteShader(vertexShader);
  gl.deleteShader(fragmentShader);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    const log = gl.getProgramInfoLog(program) ?? "unknown program link error";
    gl.deleteProgram(program);
    throw new Error(log);
  }
  return program;
}

function createWebgl2Renderer(targetCanvas: OffscreenCanvas): FrameRenderer | null {
  const gl = targetCanvas.getContext("webgl2", {
    alpha: false,
    antialias: false,
    depth: false,
    desynchronized: true,
    powerPreference: "high-performance",
    premultipliedAlpha: false,
    preserveDrawingBuffer: false,
    stencil: false,
  });
  if (!gl) return null;

  const program = linkProgram(
    gl,
    `#version 300 es
    in vec2 a_position;
    in vec2 a_texCoord;
    out vec2 v_texCoord;
    void main() {
      gl_Position = vec4(a_position, 0.0, 1.0);
      v_texCoord = a_texCoord;
    }`,
    `#version 300 es
    precision mediump float;
    uniform sampler2D u_texture;
    in vec2 v_texCoord;
    out vec4 outColor;
    void main() {
      outColor = texture(u_texture, v_texCoord);
    }`
  );
  const vao = gl.createVertexArray();
  const buffer = gl.createBuffer();
  const texture = gl.createTexture();
  if (!vao || !buffer || !texture) {
    throw new Error("WebGL2 resource allocation failed");
  }

  gl.bindVertexArray(vao);
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(
    gl.ARRAY_BUFFER,
    new Float32Array([
      -1, -1, 0, 0,
      1, -1, 1, 0,
      -1, 1, 0, 1,
      -1, 1, 0, 1,
      1, -1, 1, 0,
      1, 1, 1, 1,
    ]),
    gl.STATIC_DRAW
  );
  const positionLocation = gl.getAttribLocation(program, "a_position");
  const texCoordLocation = gl.getAttribLocation(program, "a_texCoord");
  gl.enableVertexAttribArray(positionLocation);
  gl.vertexAttribPointer(positionLocation, 2, gl.FLOAT, false, 16, 0);
  gl.enableVertexAttribArray(texCoordLocation);
  gl.vertexAttribPointer(texCoordLocation, 2, gl.FLOAT, false, 16, 8);
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.useProgram(program);
  gl.uniform1i(gl.getUniformLocation(program, "u_texture"), 0);

  return {
    backend: "webgl2",
    draw(frame: VideoFrame) {
      resizeCanvasForFrame(frame);
      gl.viewport(0, 0, targetCanvas.width, targetCanvas.height);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
      gl.texImage2D(
        gl.TEXTURE_2D,
        0,
        gl.RGBA,
        gl.RGBA,
        gl.UNSIGNED_BYTE,
        frame as unknown as TexImageSource
      );
      gl.bindVertexArray(vao);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
    },
  };
}

function createCanvas2dRenderer(targetCanvas: OffscreenCanvas): FrameRenderer | null {
  const context = targetCanvas.getContext("2d", { alpha: false });
  if (!context) return null;
  return {
    backend: "2d",
    draw(frame: VideoFrame) {
      resizeCanvasForFrame(frame);
      context.drawImage(frame, 0, 0, targetCanvas.width, targetCanvas.height);
    },
  };
}

function createFrameRenderer(targetCanvas: OffscreenCanvas): FrameRenderer | null {
  try {
    return createWebgl2Renderer(targetCanvas) ?? createCanvas2dRenderer(targetCanvas);
  } catch {
    return createCanvas2dRenderer(targetCanvas);
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

function frameAgeMs(header: WebCodecsFrameHeader) {
  return performance.timeOrigin + performance.now() - header.capture_unix_us / 1000;
}

function requestKeyframe(reason: string) {
  const now = performance.now();
  if (now - lastKeyframeRequestAt < KEYFRAME_REQUEST_COOLDOWN_MS) return;
  lastKeyframeRequestAt = now;
  try {
    socket?.send(JSON.stringify({ type: "request_keyframe", reason }));
  } catch {
    // Best-effort low-latency recovery hint.
  }
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
    renderer?.draw(frame);

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
        rendererBackend: renderer?.backend ?? "2d",
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
    rendererBackend: renderer?.backend ?? "2d",
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
  renderer = null;
  post({ type: "closed" });
}

function start(message: StartMessage) {
  stop();
  if (!("VideoDecoder" in self) || !("EncodedVideoChunk" in self)) {
    post({ type: "error", message: "WebCodecs worker VideoDecoder / EncodedVideoChunk is unavailable" });
    return;
  }

  canvas = message.canvas;
  renderer = createFrameRenderer(canvas);
  if (!renderer) {
    post({ type: "error", message: "WebCodecs worker failed to create OffscreenCanvas render context" });
    return;
  }
  applyViewport(message.viewportWidth, message.viewportHeight, message.devicePixelRatio);
  lastOutputAt = null;
  lastStatsAt = performance.now();
  framesSinceStats = 0;
  totalFrames = 0;
  droppedFrames = 0;
  lastKeyframeRequestAt = 0;
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
      const ageMs = frameAgeMs(accessUnit.header);
      const shouldDropDelta =
        !accessUnit.header.keyframe &&
        (decoder.decodeQueueSize > 2 || ageMs > STALE_DELTA_FRAME_DROP_MS);
      if (shouldDropDelta) {
        droppedFrames += 1;
        requestKeyframe(ageMs > STALE_DELTA_FRAME_DROP_MS ? "stale_delta" : "decode_queue");
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
