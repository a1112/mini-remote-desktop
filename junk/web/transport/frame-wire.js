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

export function toAnnexB(payload) {
  if (looksAnnexB(payload)) return payload;
  if (!payload.length) return payload;
  const out = new Uint8Array(payload.length + 4);
  out.set([0, 0, 0, 1], 0);
  out.set(payload, 4);
  return out;
}

export function containsIdrAnnexB(data) {
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

export class FrameReader {
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

