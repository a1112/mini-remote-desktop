const http = require('http');
const fs = require('fs');
const path = require('path');

const BASE_PORT = Number(process.env.PORT || 8080);
const MAX_PORT_TRIES = 10;

const mimeTypes = {
  '.html': 'text/html',
  '.js': 'text/javascript',
  '.css': 'text/css',
  '.json': 'application/json',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.svg': 'image/svg+xml'
};

const server = http.createServer((req, res) => {
  const requestUrl = new URL(req.url, 'http://127.0.0.1');
  let filePath = '.' + decodeURIComponent(requestUrl.pathname);
  if (filePath === './') filePath = './index.html';

  const extname = path.extname(filePath);
  const contentType = mimeTypes[extname] || 'application/octet-stream';

  fs.readFile(filePath, (error, content) => {
    if (error) {
      res.writeHead(404);
      res.end('File not found');
    } else {
      res.writeHead(200, { 'Content-Type': contentType });
      res.end(content);
    }
  });
});

function startServer(port, triesLeft = MAX_PORT_TRIES) {
  server.listen(port, () => {
    const address = server.address();
    const actualPort = address && typeof address === 'object' ? address.port : port;
    console.log(`\n========================================`);
    console.log(`  Mini Remote Desktop - Web 控制端`);
    console.log(`========================================`);
    console.log(`  访问地址: http://localhost:${actualPort}`);
    console.log(`========================================\n`);
  });

  server.once('error', (err) => {
    if (err && err.code === 'EADDRINUSE' && triesLeft > 0) {
      const nextPort = port + 1;
      console.warn(`[web] Port ${port} is in use, retrying on ${nextPort}...`);
      setTimeout(() => startServer(nextPort, triesLeft - 1), 50);
      return;
    }
    console.error('[web] Failed to start server:', err);
    process.exit(1);
  });
}

startServer(BASE_PORT);
