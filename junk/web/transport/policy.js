const DEFAULT_FAILOVER = {
  connectTimeoutMs: 1200,
  stallMs: 300,
  lossBurst: 12,
  stickyMs: 10_000,
};

export function normalizeTransport(value) {
  const v = String(value || '').trim().toLowerCase();
  if (v === 'quic') return 'webtransport';
  if (v === 'webtransport' || v === 'webrtc') return v;
  return 'webrtc';
}

export function buildWebCapabilities() {
  return {
    protocols: ['webtransport', 'webrtc'],
    platforms: ['web'],
    codecs: ['h264'],
    features: ['multi-end-compat', 'capability-negotiation', 'transport-failover']
  };
}

export function buildOfferTransportPayload(preferred = 'webtransport') {
  const normalized = normalizeTransport(preferred);
  return {
    transport: normalized,
    preferredTransport: normalized,
    availableTransports: ['webtransport', 'webrtc'],
    fallbackPolicy: { ...DEFAULT_FAILOVER },
  };
}

export function pickMediaTransport(answerPayload) {
  const selected = normalizeTransport(answerPayload?.selectedTransport);
  const webtransport = answerPayload?.webtransport || null;
  return {
    selectedTransport: selected,
    webtransport,
    quic: answerPayload?.quic || null,
  };
}

export function getPreferredTransportFromQuery(locationLike) {
  const search = locationLike?.search || '';
  const p = new URLSearchParams(search).get('transport');
  return normalizeTransport(p || 'webtransport');
}

export function canUseWebTransport(globalLike = globalThis) {
  return typeof globalLike?.WebTransport === 'function' && typeof globalLike?.VideoDecoder === 'function';
}

