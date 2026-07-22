import { describe, expect, test } from "vitest";
import { buildWsUrl } from "./ws-url";

describe("buildWsUrl", () => {
  test("uses query ws when provided", () => {
    const locationLike = {
      protocol: "http:",
      hostname: "127.0.0.1",
      search: "?ws=ws://10.0.0.2:9527"
    } as Location;
    expect(buildWsUrl(locationLike)).toBe("ws://10.0.0.2:9527");
  });

  test("builds fallback url", () => {
    const locationLike = {
      protocol: "https:",
      hostname: "demo.local",
      search: "?wsPort=1234"
    } as Location;
    expect(buildWsUrl(locationLike)).toBe("wss://demo.local:1234");
  });
});
