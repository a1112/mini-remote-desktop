import { beforeEach, describe, expect, test, vi } from "vitest";
import { SignalingClient } from "./signalingClient";

class FakeWebSocket {
  static OPEN = 1;
  static instances: FakeWebSocket[] = [];
  readyState = 1;
  onopen: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  sent: string[] = [];

  constructor(public readonly url: string) {
    FakeWebSocket.instances.push(this);
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.onclose?.({} as CloseEvent);
  }
}

describe("SignalingClient", () => {
  beforeEach(() => {
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket as unknown as typeof WebSocket);
  });

  test("registers as controller on connected", () => {
    const client = new SignalingClient("ws://127.0.0.1:9527");
    const selfIds: string[] = [];
    client.on("selfId", (id) => selfIds.push(id));
    client.connect();

    const ws = FakeWebSocket.instances[0];
    ws.onmessage?.(
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "system",
          action: "connected",
          payload: { deviceId: "abc" }
        })
      })
    );

    expect(selfIds).toEqual(["abc"]);
    expect(ws.sent.some((it) => it.includes("\"action\":\"register\""))).toBe(true);
  });

  test("emits device list", () => {
    const client = new SignalingClient("ws://127.0.0.1:9527");
    const lists: string[] = [];
    client.on("deviceList", (arr) => lists.push(arr.map((it) => it.id).join(",")));
    client.connect();
    const ws = FakeWebSocket.instances[0];

    ws.onmessage?.(
      new MessageEvent("message", {
        data: JSON.stringify({
          type: "device",
          action: "deviceList",
          payload: { deviceList: [{ id: "d1", name: "Agent", online: true }] }
        })
      })
    );

    expect(lists).toEqual(["d1"]);
  });
});
