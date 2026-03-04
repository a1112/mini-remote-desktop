const DEFAULT_BACKEND_WS = 'ws://198.18.0.1:9527';

export function buildWsUrl(locationLike, port = 9527) {
  const search = locationLike?.search || '';
  const params = new URLSearchParams(search);
  const wsOverride = params.get('ws');
  if (wsOverride && /^wss?:\/\//i.test(wsOverride)) {
    return wsOverride;
  }

  const hostname = (locationLike?.hostname || '').trim();
  if (!hostname || hostname === 'localhost' || hostname === '127.0.0.1') {
    return DEFAULT_BACKEND_WS;
  }

  return `ws://${hostname}:${port}`;
}
