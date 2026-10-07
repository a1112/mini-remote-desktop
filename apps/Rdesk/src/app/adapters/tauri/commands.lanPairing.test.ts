import { afterEach, describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "@/test/mocks/tauri";
import * as adapter from "./commands";

describe("installed UI LAN pairing IPC contract", () => {
  afterEach(() => {
    delete (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__;
    vi.unstubAllGlobals();
  });
  it("lists signed candidates through the typed product IPC", async () => {
    expect(adapter.ipcListLanPairingCandidates).toBeTypeOf("function");
    const invoke = getMockInvoke();
    invoke.mockResolvedValue({ type: "LanPairingCandidateList", candidates: [] });
    expect(await adapter.ipcListLanPairingCandidates()).toEqual({ ok: true, value: [] });
    expect(invoke).toHaveBeenCalledWith("ipc_secure_remote", {
      request: { type: "ListLanPairingCandidates" },
    });
  });

  it("binds the approval to the service candidate without sending public-key evidence", async () => {
    expect(adapter.ipcApproveLanPairing).toBeTypeOf("function");
    const approval = {
      candidate_id: "candidate-1", device_id: "peer-1", peer_key_id: "a".repeat(64),
      key_epoch: "18446744073709551615", discovery_endpoint: "192.168.1.241:21116",
      permission_ceiling: ["screen.view" as const],
    };
    const device = { peer_key_id: approval.peer_key_id, state: "trusted", permission_ceiling: ["screen.view"] };
    const invoke = getMockInvoke();
    invoke.mockResolvedValue({ type: "TrustedDeviceUpdated", device });
    expect(await adapter.ipcApproveLanPairing(approval)).toEqual({ ok: true, value: device });
    expect(invoke).toHaveBeenCalledWith("ipc_secure_remote", {
      request: { type: "ApproveLanPairing", approval },
    });
  });

  it("rejects browser bridge pairing locally without sending an IPC write", async () => {
    (window as Window & { __MRD_FORCE_WEB_BRIDGE__?: boolean }).__MRD_FORCE_WEB_BRIDGE__ = true;
    const fetch = vi.fn();
    vi.stubGlobal("fetch", fetch);
    const result = await adapter.ipcApproveLanPairing({
      candidate_id: "candidate-1", device_id: "peer-1", peer_key_id: "a".repeat(64),
      key_epoch: "1", discovery_endpoint: "192.168.1.241:21116", permission_ceiling: ["screen.view"],
    });
    expect(result).toEqual({ ok: false, error: {
      code: "E_INSTALLED_UI_REQUIRED", message: "请在本机已安装的客户端中确认局域网配对",
    } });
    expect(getMockInvoke()).not.toHaveBeenCalled();
    expect(fetch).not.toHaveBeenCalled();
  });

  it("preserves service actor rejection without retrying the legacy trust mutation", async () => {
    const invoke = getMockInvoke();
    invoke.mockResolvedValue({ type: "Error", code: "E_INSTALLED_UI_REQUIRED", message: "installed UI identity required" });
    expect(await adapter.ipcApproveLanPairing({
      candidate_id: "candidate-1", device_id: "peer-1", peer_key_id: "a".repeat(64),
      key_epoch: "1", discovery_endpoint: "192.168.1.241:21116", permission_ceiling: ["screen.view"],
    })).toEqual({ ok: false, error: { code: "E_INSTALLED_UI_REQUIRED", message: "installed UI identity required" } });
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});
