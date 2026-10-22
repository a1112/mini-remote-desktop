import { afterEach, describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "@/test/mocks/tauri";
import { ipcBindPublicDevice, ipcUnbindPublicDevice, ipcPublicServerStatus } from "./commands";
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

  it("sends an ephemeral user credential only through the native binding commands", async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    const invoke = getMockInvoke();
    invoke.mockResolvedValue(undefined);
    const fetch = vi.fn();
    vi.stubGlobal("fetch", fetch);
    expect(await ipcBindPublicDevice("user.access.token")).toEqual({ ok: true, value: undefined });
    expect(await ipcUnbindPublicDevice("user.access.token")).toEqual({ ok: true, value: undefined });
    expect(invoke).toHaveBeenCalledWith("ipc_bind_public_device", { userToken: "user.access.token" });
    expect(invoke).toHaveBeenCalledWith("ipc_unbind_public_device", { userToken: "user.access.token" });
    expect(fetch).not.toHaveBeenCalled();
  });
});
