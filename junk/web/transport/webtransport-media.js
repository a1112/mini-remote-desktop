import { FrameReader, containsIdrAnnexB, toAnnexB } from './frame-wire.js';

export class WebTransportMediaClient {
  constructor({ endpoint, onStats, onLog, canvas }) {
    this.endpoint = endpoint;
    this.onStats = onStats || (() => {});
    this.onLog = onLog || (() => {});
    this.canvas = canvas;
    this.transport = null;
    this.readerAbort = false;
    this.decoder = null;
    this.ctx = this.canvas.getContext('2d', { alpha: false, desynchronized: true });
    this.lastFrameAt = 0;
    this.bytes = 0;
    this.frameCount = 0;
    this._initDecoder();
  }

  _initDecoder() {
    this.decoder = new VideoDecoder({
      output: (frame) => {
        this.lastFrameAt = performance.now();
        if (this.canvas.width !== frame.displayWidth || this.canvas.height !== frame.displayHeight) {
          this.canvas.width = frame.displayWidth;
          this.canvas.height = frame.displayHeight;
        }
        this.ctx.drawImage(frame, 0, 0, this.canvas.width, this.canvas.height);
        frame.close();
      },
      error: (e) => this.onLog('warn', `webcodec decode error: ${e?.message || e}`),
    });
    this.decoder.configure({
      codec: 'avc1.64001f',
      optimizeForLatency: true,
      hardwareAcceleration: 'prefer-hardware',
    });
  }

  async connect(timeoutMs = 1200) {
    const hash = Array.isArray(this.endpoint.certFingerprintSha256)
      ? new Uint8Array(this.endpoint.certFingerprintSha256)
      : null;
    const connectTask = (async () => {
      this.transport = new WebTransport(this.endpoint.url, {
        serverCertificateHashes: hash
          ? [{ algorithm: 'sha-256', value: hash.buffer }]
          : undefined,
      });
      await this.transport.ready;
      this.onLog('info', `WebTransport connected: ${this.endpoint.url}`);
      this._startIncomingLoop();
    })();
    await Promise.race([
      connectTask,
      new Promise((_, reject) => setTimeout(() => reject(new Error('webtransport connect timeout')), timeoutMs)),
    ]);
  }

  async _startIncomingLoop() {
    this.readerAbort = false;
    const incoming = this.transport.incomingUnidirectionalStreams;
    for await (const stream of incoming) {
      if (this.readerAbort) return;
      this._consumeStream(stream).catch((e) => this.onLog('warn', `stream consume failed: ${e?.message || e}`));
    }
  }

  async _consumeStream(stream) {
    const reader = stream.getReader();
    const parser = new FrameReader();
    while (!this.readerAbort) {
      const { value, done } = await reader.read();
      if (done) break;
      if (!value || !value.length) continue;
      this.bytes += value.length;
      const frames = parser.push(value);
      for (const frame of frames) {
        const annexb = toAnnexB(frame.payload);
        const isKey = containsIdrAnnexB(annexb);
        const tsUs = Number(frame.txUnixUs > BigInt(Number.MAX_SAFE_INTEGER) ? BigInt(0) : frame.txUnixUs);
        const chunk = new EncodedVideoChunk({
          type: isKey ? 'key' : 'delta',
          timestamp: tsUs,
          data: annexb,
        });
        this.decoder.decode(chunk);
        this.frameCount += 1;
        this.onStats({
          frameCount: this.frameCount,
          bytes: this.bytes,
          lastFrameAt: this.lastFrameAt,
          seq: Number(frame.seq),
          transport: 'webtransport',
        });
      }
    }
  }

  async close() {
    this.readerAbort = true;
    try {
      await this.decoder?.flush();
    } catch (_) {}
    this.decoder?.close();
    this.decoder = null;
    if (this.transport) {
      try {
        this.transport.close();
      } catch (_) {}
      this.transport = null;
    }
  }
}
