export function buildWsUrl(locationLike: Location): string {
  const params = new URLSearchParams(locationLike.search);
  const qs = params.get("ws");
  if (qs) {
    return qs;
  }

  const protocol = locationLike.protocol === "https:" ? "wss:" : "ws:";
  const host = locationLike.hostname || "127.0.0.1";
  const port = params.get("wsPort") ?? "9527";
  return `${protocol}//${host}:${port}`;
}
