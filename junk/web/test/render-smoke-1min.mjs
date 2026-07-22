import { chromium } from 'playwright';

const WEB_URL = process.env.WEB_URL || 'http://127.0.0.1:8080/?transport=webrtc';
const DURATION_MS = Number(process.env.RENDER_SMOKE_MS || 60_000);
const SAMPLE_MS = 1000;

function parseNum(text) {
  const n = Number(String(text || '').trim());
  return Number.isFinite(n) ? n : null;
}

function summarize(values) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const sum = values.reduce((a, b) => a + b, 0);
  const p50 = sorted[Math.floor(sorted.length * 0.5)];
  const p95 = sorted[Math.floor(sorted.length * 0.95)];
  return {
    min: sorted[0],
    max: sorted[sorted.length - 1],
    avg: sum / values.length,
    p50,
    p95,
    n: values.length,
  };
}

async function main() {
  const browser = await chromium.launch({
    headless: false,
    channel: 'msedge',
  });
  const page = await browser.newPage();
  const consoleLog = [];
  page.on('console', (m) => {
    const line = `[${m.type()}] ${m.text()}`;
    consoleLog.push(line);
    if (consoleLog.length > 300) consoleLog.shift();
  });

  await page.goto(WEB_URL, { waitUntil: 'domcontentloaded', timeout: 15_000 });
  await page.waitForTimeout(1500);

  await page.waitForSelector('#deviceList .device-item', { timeout: 20_000 });
  await page.click('#deviceList .device-item');
  await page.click('#btnConnect');

  await page.waitForFunction(
    () => {
      const v = document.getElementById('remoteVideo');
      const c = document.getElementById('remoteCanvas');
      return (v && getComputedStyle(v).display !== 'none') || (c && getComputedStyle(c).display !== 'none');
    },
    null,
    { timeout: 30_000 },
  );

  const start = Date.now();
  const fpsVals = [];
  const latencyVals = [];
  const bitrateVals = [];
  const playbackFpsVals = [];
  const snapshots = [];
  let lastVideoTime = null;
  let lastSampleTs = null;

  while (Date.now() - start < DURATION_MS) {
    const snap = await page.evaluate(() => {
      const fps = document.getElementById('fps')?.textContent ?? '';
      const latency = document.getElementById('latency')?.textContent ?? '';
      const bitrate = document.getElementById('bitrate')?.textContent ?? '';
      const status = document.getElementById('statusText')?.textContent ?? '';
      const video = document.getElementById('remoteVideo');
      return {
        fps,
        latency,
        bitrate,
        status,
        videoCurrentTime: video?.currentTime ?? 0,
        videoReadyState: video?.readyState ?? 0,
        videoWidth: video?.videoWidth ?? 0,
        videoHeight: video?.videoHeight ?? 0,
      };
    });
    snapshots.push({ t: Date.now() - start, ...snap });

    const fps = parseNum(snap.fps);
    const latency = parseNum(snap.latency);
    const bitrate = parseNum(snap.bitrate);
    if (fps !== null) fpsVals.push(fps);
    if (latency !== null) latencyVals.push(latency);
    if (bitrate !== null) bitrateVals.push(bitrate);
    const now = Date.now();
    if (lastVideoTime !== null && lastSampleTs !== null) {
      const dt = Math.max((now - lastSampleTs) / 1000, 1e-3);
      const dv = Math.max(0, (snap.videoCurrentTime || 0) - lastVideoTime);
      if (dv > 0) {
        playbackFpsVals.push(dv / dt);
      }
    }
    lastVideoTime = snap.videoCurrentTime || 0;
    lastSampleTs = now;

    await page.waitForTimeout(SAMPLE_MS);
  }

  const report = {
    webUrl: WEB_URL,
    durationMs: DURATION_MS,
    fps: summarize(fpsVals),
    latencyMs: summarize(latencyVals),
    bitrateMbps: summarize(bitrateVals),
    playbackFps: summarize(playbackFpsVals),
    lastSnapshot: snapshots[snapshots.length - 1] || null,
    consoleTail: consoleLog.slice(-30),
  };
  // Keep output easy to parse by shell
  console.log(JSON.stringify(report, null, 2));

  await browser.close();
}

main().catch((e) => {
  console.error(`[render-smoke] failed: ${e?.stack || e}`);
  process.exit(1);
});
