const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const { loadRuntimeConfig } = require('../config');

const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'mini-remote-agent-test-'));
const goodConfigPath = path.join(tmpDir, 'good.json');
const badConfigPath = path.join(tmpDir, 'bad.json');

fs.writeFileSync(goodConfigPath, JSON.stringify({
  wsUrl: 'ws://192.168.1.10:9527',
  capture: {
    fps: 45,
    minWidth: 1024,
    maxWidth: 2560,
    minHeight: 720,
    maxHeight: 1440
  }
}), 'utf8');

fs.writeFileSync(badConfigPath, JSON.stringify({
  capture: { fps: 999 }
}), 'utf8');

const good = loadRuntimeConfig(goodConfigPath);
assert.equal(good.wsUrl, 'ws://192.168.1.10:9527');
assert.equal(good.capture.fps, 45);
assert.equal(good.capture.maxWidth, 2560);

const bad = loadRuntimeConfig(badConfigPath);
assert.equal(bad.capture.fps, 30);

const missing = loadRuntimeConfig(path.join(tmpDir, 'missing.json'));
assert.equal(missing.wsUrl, 'ws://localhost:9527');
assert.equal(missing.capture.fps, 30);

console.log('config tests passed');
