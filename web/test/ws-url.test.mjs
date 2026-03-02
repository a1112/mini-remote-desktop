import assert from 'node:assert/strict';
import { buildWsUrl } from '../ws-url.js';

assert.equal(buildWsUrl({ protocol: 'file:', hostname: '' }), 'ws://localhost:9527');
assert.equal(buildWsUrl({ protocol: 'http:', hostname: '192.168.1.8' }), 'ws://192.168.1.8:9527');

console.log('ws-url tests passed');
