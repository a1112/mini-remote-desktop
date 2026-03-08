import assert from 'node:assert/strict';
import { parseListeningUrl } from './auto-debug.mjs';

const sample = `\n[web] Port 8080 is in use, retrying on 8081...\n访问地址: http://localhost:8081\n`;
assert.equal(parseListeningUrl(sample), 'http://localhost:8081');

console.log('auto-debug test passed');
