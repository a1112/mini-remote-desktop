export function buildWsUrl(locationLike, port = 9527) {
  const hostname = locationLike?.hostname || 'localhost';
  return `ws://${hostname}:${port}`;
}
