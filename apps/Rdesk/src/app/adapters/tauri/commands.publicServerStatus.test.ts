import { afterEach, describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "@/test/mocks/tauri";
import { ipcPublicServerStatus } from "./commands";
import { resetServiceBridgeConfigForTest } from "../serviceBridge/client";

const status = {
  service_running: true,
  api_url: "https://api.example/api/v1",
  api_reachable: true,
  device_registered: true,
  device_id: "0123456789",
  device_name: "Office PC",
  signaling_state: "authenticated",
  reconnect_attempt: 0,
  last_connected_at_ms: 1000,
  last_error: null,
};

describe("public server management command adapter", () => {
  afterEach(() => {
    delete (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__;
    resetServiceBridgeConfigForTest();
    vi.unstubAllGlobals();
  });

  it("reads the dedicated secret-free native management command", async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue(status);
    expect(await ipcPublicServerStatus()).toEqual({ ok: true, value: status });
    expect(invoke).toHaveBeenCalledWith("ipc_public_server_status", undefined);
  });

  it("supports the explicit web bridge without using the protected runtime snapshot", async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({ response: { type: "PublicServerStatus", status } }), { status: 200, headers: { "Content-Type": "application/json" } }));
    vi.stubGlobal("fetch", fetch);
    expect(await ipcPublicServerStatus()).toEqual({ ok: true, value: status });
    expect(JSON.parse(fetch.mock.calls[0]?.[1]?.body ?? "{}")).toEqual({ request: { type: "GetPublicServerStatus" } });
  });
});
