import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';

export function parseListeningUrl(text) {
  const m = text.match(/访问地址:\s*(https?:\/\/[^\s]+)/);
  return m ? m[1].trim() : null;
}

function startWebServer({ timeoutMs = 10000 } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ['server.js'], {
      cwd: new URL('..', import.meta.url),
      stdio: ['ignore', 'pipe', 'pipe'],
      env: {
        ...process.env,
        PORT: '0',
      },
    });

    let settled = false;
    let logs = '';

    const finalize = (fn) => (value) => {
      if (settled) return;
      settled = true;
      fn(value);
    };

    const doneResolve = finalize(resolve);
    const doneReject = finalize(reject);

    const timer = setTimeout(() => {
      doneReject(new Error(`web server start timeout after ${timeoutMs}ms\nlogs:\n${logs}`));
    }, timeoutMs);

    const onData = (chunk) => {
      const text = chunk.toString();
      logs += text;
      const url = parseListeningUrl(logs);
      if (url) {
        clearTimeout(timer);
        doneResolve({ child, url, logs });
      }
    };

    child.stdout.on('data', onData);
    child.stderr.on('data', onData);

    child.on('error', (err) => {
      clearTimeout(timer);
      doneReject(err);
    });

    child.on('exit', (code) => {
      if (!settled && code !== 0) {
        clearTimeout(timer);
        doneReject(new Error(`web server exited code=${code}\nlogs:\n${logs}`));
      }
    });
  });
}

async function checkHttp(url, path, marker) {
  const target = `${url}${path}`;
  const res = await fetch(target);
  const body = await res.text();
  assert.equal(res.status, 200, `${target} status should be 200`);
  assert.ok(body.includes(marker), `${target} should include marker: ${marker}`);
  return {
    target,
    status: res.status,
    size: body.length,
  };
}

async function runPerfTest() {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, ['test/webtransport-perf.test.mjs'], {
      cwd: new URL('..', import.meta.url),
      stdio: ['ignore', 'pipe', 'pipe'],
    });

    let out = '';
    let err = '';
    child.stdout.on('data', (c) => {
      out += c.toString();
    });
    child.stderr.on('data', (c) => {
      err += c.toString();
    });

    child.on('exit', (code) => {
      resolve({
        ok: code === 0,
        code,
        out: out.trim(),
        err: err.trim(),
      });
    });
  });
}

async function main() {
  const summary = {
    startedAt: new Date().toISOString(),
    server: null,
    checks: [],
    perf: null,
    ok: false,
  };

  let server = null;
  try {
    server = await startWebServer();
    summary.server = {
      url: server.url,
      note: 'auto-started server.js',
    };

    summary.checks.push(await checkHttp(server.url, '/', '<title>Mini Remote Desktop'));
    summary.checks.push(await checkHttp(server.url, '/app.js', 'connectWebSocket')); 
    summary.checks.push(await checkHttp(server.url, '/ws-url.js', 'DEFAULT_BACKEND_WS'));

    summary.perf = await runPerfTest();
    summary.ok = summary.checks.length === 3 && summary.perf.ok;
  } catch (err) {
    summary.error = err?.stack || String(err);
    summary.ok = false;
  } finally {
    if (server?.child && !server.child.killed) {
      server.child.kill('SIGTERM');
      await delay(80);
      if (!server.child.killed) {
        server.child.kill('SIGKILL');
      }
    }
  }

  console.log(JSON.stringify(summary, null, 2));
  if (!summary.ok) process.exit(1);
}

if (process.argv[1] && /auto-debug\.mjs$/i.test(process.argv[1])) {
  main();
}
