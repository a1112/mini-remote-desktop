function readU32BE(buf, off) {
  const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  return dv.getUint32(off, false);
}

function readU64BE(buf, off) {
  const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const hi = BigInt(dv.getUint32(off, false));
  const lo = BigInt(dv.getUint32(off + 4, false));
  return (hi << 32n) | lo;
}

function looksAnnexB(data) {
  if (data.length < 4) return false;
  return (
    (data[0] === 0 && data[1] === 0 && data[2] === 1) ||
    (data[0] === 0 && data[1] === 0 && data[2] === 0 && data[3] === 1)
  );
}

function toAnnexB(payload) {
  if (looksAnnexB(payload)) return payload;
  if (!payload.length) return payload;
  const out = new Uint8Array(payload.length + 4);
  out.set([0, 0, 0, 1], 0);
  out.set(payload, 4);
  return out;
}

function containsIdrAnnexB(data) {
  for (let i = 0; i + 4 < data.length; i++) {
    let sc = 0;
    if (data[i] === 0 && data[i + 1] === 0 && data[i + 2] === 1) sc = 3;
    if (data[i] === 0 && data[i + 1] === 0 && data[i + 2] === 0 && data[i + 3] === 1) sc = 4;
    if (!sc) continue;
    const hdr = i + sc;
    if (hdr < data.length && (data[hdr] & 0x1f) === 5) return true;
    i = hdr;
  }
  return false;
}

class FrameReader {
  constructor() {
    this.pending = new Uint8Array(0);
  }

  push(chunk) {
    const merged = new Uint8Array(this.pending.length + chunk.length);
    merged.set(this.pending, 0);
    merged.set(chunk, this.pending.length);
    this.pending = merged;
    return this._drain();
  }

  _drain() {
    const out = [];
    let off = 0;
    while (this.pending.length - off >= 20) {
      const len = readU32BE(this.pending, off);
      if (this.pending.length - off < 20 + len) break;
      const seq = readU64BE(this.pending, off + 4);
      const txUnixUs = readU64BE(this.pending, off + 12);
      const payload = this.pending.slice(off + 20, off + 20 + len);
      out.push({ seq, txUnixUs, payload });
      off += 20 + len;
    }
    this.pending = this.pending.slice(off);
    return out;
  }
}

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
    const connectTask = (async () => {
      this.transport = new WebTransport(this.endpoint.url, {
        serverCertificateHashes: this.endpoint.certFingerprintSha256
          ? [{ algorithm: 'sha-256', value: this.endpoint.certFingerprintSha256 }]
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

