import assert from 'node:assert/strict';
import {
  buildOfferTransportPayload,
  buildWebCapabilities,
  normalizeTransport,
  pickMediaTransport,
  getPreferredTransportFromQuery,
} from '../transport/policy.js';

assert.equal(normalizeTransport('quic'), 'webtransport');
assert.equal(normalizeTransport('webtransport'), 'webtransport');
assert.equal(normalizeTransport('webrtc'), 'webrtc');
assert.equal(normalizeTransport('bad-value'), 'webrtc');

const caps = buildWebCapabilities();
assert.deepEqual(caps.protocols, ['webtransport', 'webrtc']);
assert.equal(caps.platforms[0], 'web');

const offerPayload = buildOfferTransportPayload('quic');
assert.equal(offerPayload.transport, 'webtransport');
assert.equal(offerPayload.preferredTransport, 'webtransport');
assert.deepEqual(offerPayload.availableTransports, ['webtransport', 'webrtc']);
assert.equal(typeof offerPayload.fallbackPolicy.connectTimeoutMs, 'number');

const picked = pickMediaTransport({
  selectedTransport: 'quic',
  webtransport: { url: 'https://198.18.0.1:8443/mrd' },
});
assert.equal(picked.selectedTransport, 'webtransport');
assert.equal(picked.webtransport.url, 'https://198.18.0.1:8443/mrd');

assert.equal(
  getPreferredTransportFromQuery({ search: '?transport=quic' }),
  'webtransport'
);
assert.equal(
  getPreferredTransportFromQuery({ search: '?transport=webrtc' }),
  'webrtc'
);
assert.equal(
  getPreferredTransportFromQuery({ search: '' }),
  'webtransport'
);

console.log('transport-policy tests passed');

