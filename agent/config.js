const fs = require('node:fs');
const path = require('node:path');

const DEFAULT_CONFIG = {
  wsUrl: 'ws://localhost:9527',
  capture: {
    fps: 30,
    minWidth: 1280,
    maxWidth: 1920,
    minHeight: 720,
    maxHeight: 1080
  }
};

function toPositiveInt(value, fallback) {
  const n = Number(value);
  if (!Number.isFinite(n)) return fallback;
  const i = Math.floor(n);
  return i > 0 ? i : fallback;
}

function normalizeConfig(input = {}) {
  const wsUrl = typeof input.wsUrl === 'string' && input.wsUrl.trim()
    ? input.wsUrl.trim()
    : DEFAULT_CONFIG.wsUrl;

  const capture = input.capture || {};
  const fps = toPositiveInt(capture.fps, DEFAULT_CONFIG.capture.fps);
  const safeFps = fps >= 1 && fps <= 120 ? fps : DEFAULT_CONFIG.capture.fps;

  return {
    wsUrl,
    capture: {
      fps: safeFps,
      minWidth: toPositiveInt(capture.minWidth, DEFAULT_CONFIG.capture.minWidth),
      maxWidth: toPositiveInt(capture.maxWidth, DEFAULT_CONFIG.capture.maxWidth),
      minHeight: toPositiveInt(capture.minHeight, DEFAULT_CONFIG.capture.minHeight),
      maxHeight: toPositiveInt(capture.maxHeight, DEFAULT_CONFIG.capture.maxHeight)
    }
  };
}

function loadRuntimeConfig(configPath = path.join(__dirname, 'config.json')) {
  try {
    if (!fs.existsSync(configPath)) return { ...DEFAULT_CONFIG, capture: { ...DEFAULT_CONFIG.capture } };
    const raw = fs.readFileSync(configPath, 'utf8').replace(/^\uFEFF/, '');
    const parsed = JSON.parse(raw);
    return normalizeConfig(parsed);
  } catch (err) {
    console.warn('[Config] 配置加载失败，使用默认配置:', err.message);
    return { ...DEFAULT_CONFIG, capture: { ...DEFAULT_CONFIG.capture } };
  }
}

module.exports = {
  DEFAULT_CONFIG,
  loadRuntimeConfig
};
