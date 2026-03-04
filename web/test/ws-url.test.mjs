import assert from 'node:assert/strict';
import { buildWsUrl } from '../ws-url.js';

assert.equal(buildWsUrl({ protocol: 'file:', hostname: '' }), 'ws://198.18.0.1:9527');
assert.equal(buildWsUrl({ protocol: 'http:', hostname: 'localhost' }), 'ws://198.18.0.1:9527');
assert.equal(buildWsUrl({ protocol: 'http:', hostname: '192.168.1.8' }), 'ws://192.168.1.8:9527');
assert.equal(
  buildWsUrl({ protocol: 'http:', hostname: 'localhost', search: '?ws=ws://127.0.0.1:9527' }),
  'ws://127.0.0.1:9527'
);

console.log('ws-url tests passed');
