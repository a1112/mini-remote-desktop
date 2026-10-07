import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LanPairingCandidate } from "../adapters/tauri/types";
import { LanPairingPanel } from "./LanPairingPanel";

const commands = vi.hoisted(() => ({
  ipcListLanPairingCandidates: vi.fn(), ipcApproveLanPairing: vi.fn(),
  ipcRefreshLanDiscovery: vi.fn(),
}));
vi.mock("../adapters/tauri", () => commands);

const candidate = (overrides: Partial<LanPairingCandidate> = {}): LanPairingCandidate => ({
  candidate_id: "candidate-current", device_id: "lan-MSLGKRSZGOVODD",
  device_name: "MS-LGKRSZGOVODD", peer_key_id: "86e02445b5059bc92cbc86e3fae3ac6d262e97d0b9407b91ac7331992780c638",
  key_epoch: "18446744073709551615", discovery_endpoint: "192.168.1.241:21116",
  expires_at_ms: Date.now() + 30_000, permission_ceiling: ["screen.view"], ...overrides,
});

beforeEach(() => {
  commands.ipcListLanPairingCandidates.mockReset().mockResolvedValue({ ok: true, value: [candidate()] });
  commands.ipcApproveLanPairing.mockReset().mockResolvedValue({ ok: true, value: { state: "trusted" } });
  commands.ipcRefreshLanDiscovery.mockReset().mockResolvedValue({ ok: true, value: {} });
});

async function openConfirmation() {
  fireEvent.click(await screen.findByRole("button", { name: "核对并配对 MS-LGKRSZGOVODD" }));
  return screen.getByRole("alertdialog");
}

describe("explicit first LAN pairing panel", () => {
  it("displays the exact signed identity and sends only screen.view after confirmation", async () => {
    const peer = candidate();
    commands.ipcListLanPairingCandidates.mockResolvedValue({ ok: true, value: [peer] });
    const onPaired = vi.fn();
    render(<LanPairingPanel onPaired={onPaired} />);
    const dialog = await openConfirmation();
    expect(within(dialog).getByText(peer.device_name)).toBeInTheDocument();
    expect(within(dialog).getByText(peer.device_id)).toBeInTheDocument();
    expect(within(dialog).getByText(peer.discovery_endpoint)).toBeInTheDocument();
    expect(within(dialog).getByText(peer.peer_key_id)).toBeInTheDocument();
    expect(within(dialog).getByText(peer.key_epoch)).toBeInTheDocument();
    expect(within(dialog).getByText(new Date(peer.expires_at_ms).toLocaleString())).toBeInTheDocument();
    expect(within(dialog).getByText(/每次远程会话仍需在被控设备上确认/)).toBeInTheDocument();
    expect(commands.ipcApproveLanPairing).not.toHaveBeenCalled();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "确认配对，仅屏幕查看" }));
    await waitFor(() => expect(commands.ipcApproveLanPairing).toHaveBeenCalledTimes(1));
    expect(commands.ipcApproveLanPairing).toHaveBeenCalledWith({
      candidate_id: peer.candidate_id, device_id: peer.device_id, peer_key_id: peer.peer_key_id,
      key_epoch: peer.key_epoch, discovery_endpoint: peer.discovery_endpoint,
      permission_ceiling: ["screen.view"],
    });
    await waitFor(() => expect(commands.ipcRefreshLanDiscovery).toHaveBeenCalledTimes(1));
    expect(onPaired).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("allows cancellation without granting machine trust or opening a session", async () => {
    render(<LanPairingPanel />);
    const dialog = await openConfirmation();
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "取消" })).toHaveFocus());
    fireEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(commands.ipcApproveLanPairing).not.toHaveBeenCalled();
    expect(commands.ipcRefreshLanDiscovery).not.toHaveBeenCalled();
  });

  it("does not offer approval for an expired candidate", async () => {
    commands.ipcListLanPairingCandidates.mockResolvedValue({ ok: true, value: [candidate({ expires_at_ms: Date.now() - 1 })] });
    render(<LanPairingPanel />);
    const approve = await screen.findByRole("button", { name: "核对并配对 MS-LGKRSZGOVODD" });
    expect(approve).toBeDisabled();
    fireEvent.click(approve);
    expect(commands.ipcApproveLanPairing).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("expires a confirmation while it is open and checks the deadline again on click", async () => {
    const peer = candidate();
    commands.ipcListLanPairingCandidates.mockResolvedValue({ ok: true, value: [peer] });
    render(<LanPairingPanel />);
    const dialog = await openConfirmation();
    const approve = within(dialog).getByRole("button", { name: "确认配对，仅屏幕查看" });
    const clock = vi.spyOn(Date, "now").mockReturnValue(peer.expires_at_ms);
    try {
      fireEvent.click(approve);
      expect(commands.ipcApproveLanPairing).not.toHaveBeenCalled();
      expect(within(dialog).getByRole("alert")).toHaveTextContent("已过期");
      expect(approve).toBeDisabled();
    } finally { clock.mockRestore(); }
  });

  it("updates an open dialog when the displayed lease reaches its expiry", async () => {
    vi.useFakeTimers();
    try {
      const peer = candidate();
      commands.ipcListLanPairingCandidates.mockResolvedValue({ ok: true, value: [peer] });
      render(<LanPairingPanel />);
      await act(async () => { await Promise.resolve(); });
      fireEvent.click(screen.getByRole("button", { name: "核对并配对 MS-LGKRSZGOVODD" }));
      const dialog = screen.getByRole("alertdialog");
      act(() => { vi.setSystemTime(peer.expires_at_ms + 1); vi.advanceTimersByTime(1_000); });
      expect(within(dialog).getByRole("button", { name: "确认配对，仅屏幕查看" })).toBeDisabled();
    } finally { vi.useRealTimers(); }
  });

  it("shows service rejection without a fallback trust mutation", async () => {
    commands.ipcApproveLanPairing.mockResolvedValue({ ok: false, error: { code: "E_LAN_PAIRING_STALE", message: "候选签名已变化" } });
    render(<LanPairingPanel />);
    const dialog = await openConfirmation();
    fireEvent.click(within(dialog).getByRole("button", { name: "确认配对，仅屏幕查看" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("候选签名已变化");
    expect(commands.ipcRefreshLanDiscovery).not.toHaveBeenCalled();
    expect(commands.ipcApproveLanPairing).toHaveBeenCalledTimes(1);
  });

  it("shows candidate query errors without inventing a candidate", async () => {
    commands.ipcListLanPairingCandidates.mockResolvedValue({ ok: false, error: { message: "本地服务不可用" } });
    render(<LanPairingPanel />);
    expect(await screen.findByRole("alert")).toHaveTextContent("本地服务不可用");
    expect(screen.queryByRole("button", { name: /核对并配对/ })).not.toBeInTheDocument();
    expect(commands.ipcApproveLanPairing).not.toHaveBeenCalled();
  });

  it.each([
    { permission_ceiling: ["screen.view", "input.pointer"] },
    { expires_at_ms: Number.NaN },
    { peer_key_id: "" },
    { key_epoch: "18446744073709551616" },
  ] as Partial<LanPairingCandidate>[]) ("cannot approve malformed or widened candidate metadata: %j", async (invalid) => {
    commands.ipcListLanPairingCandidates.mockResolvedValue({ ok: true, value: [candidate(invalid)] });
    render(<LanPairingPanel />);
    const approve = await screen.findByRole("button", { name: "核对并配对 MS-LGKRSZGOVODD" });
    expect(approve).toBeDisabled();
    fireEvent.click(approve);
    expect(commands.ipcApproveLanPairing).not.toHaveBeenCalled();
  });

  it("latches the first explicit confirmation while the service is pending", async () => {
    let complete!: (value: unknown) => void;
    commands.ipcApproveLanPairing.mockReturnValue(new Promise(resolve => { complete = resolve; }));
    render(<LanPairingPanel />);
    const dialog = await openConfirmation();
    const approve = within(dialog).getByRole("button", { name: "确认配对，仅屏幕查看" });
    act(() => { fireEvent.click(approve); fireEvent.click(approve); });
    expect(commands.ipcApproveLanPairing).toHaveBeenCalledTimes(1);
    expect(approve).toBeDisabled();
    await act(async () => { complete({ ok: false, error: { message: "服务繁忙" } }); });
  });

  it("refreshes signed discovery only after the user's refresh action", async () => {
    const user = userEvent.setup();
    render(<LanPairingPanel />);
    await screen.findByRole("button", { name: "核对并配对 MS-LGKRSZGOVODD" });
    expect(commands.ipcRefreshLanDiscovery).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "刷新配对候选" }));
    expect(commands.ipcRefreshLanDiscovery).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(commands.ipcListLanPairingCandidates).toHaveBeenCalledTimes(2));
  });

  it("keeps a slowly reviewed confirmation usable only while the exact binding has continuous leases", async () => {
    vi.useFakeTimers();
    try {
      const peer = candidate({ expires_at_ms: Date.now() + 3_000 });
      commands.ipcListLanPairingCandidates.mockImplementation(async () => ({
        ok: true, value: [{ ...peer, expires_at_ms: Date.now() + 3_000 }],
      }));
      render(<LanPairingPanel />);
      await act(async () => { await Promise.resolve(); });
      fireEvent.click(screen.getByRole("button", { name: "核对并配对 MS-LGKRSZGOVODD" }));
      await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
      const dialog = screen.getByRole("alertdialog");
      const confirm = within(dialog).getByRole("button", { name: "确认配对，仅屏幕查看" });
      expect(confirm).toBeEnabled();
      expect(within(dialog).getByText(new Date(Date.now() + 3_000).toLocaleString())).toBeInTheDocument();
      expect(commands.ipcListLanPairingCandidates).toHaveBeenCalledTimes(6);
      expect(commands.ipcRefreshLanDiscovery).not.toHaveBeenCalled();
      expect(commands.ipcApproveLanPairing).not.toHaveBeenCalled();
      fireEvent.click(confirm);
      await act(async () => { await Promise.resolve(); });
      expect(commands.ipcApproveLanPairing).toHaveBeenCalledTimes(1);
    } finally { vi.useRealTimers(); }
  });

  it.each([
    { candidate_id: "replacement-candidate" }, { device_id: "replacement-device" },
    { device_name: "Renamed machine" }, { peer_key_id: "a".repeat(64) },
    { key_epoch: "2" }, { discovery_endpoint: "192.168.1.242:21116" },
    { permission_ceiling: ["screen.view", "input.pointer"] },
  ] as Partial<LanPairingCandidate>[]) ("invalidates the user's old confirmation when any displayed binding changes: %j", async changed => {
    vi.useFakeTimers();
    try {
      const peer = candidate();
      commands.ipcListLanPairingCandidates.mockResolvedValueOnce({ ok: true, value: [peer] })
        .mockResolvedValue({ ok: true, value: [{ ...peer, ...changed, expires_at_ms: Date.now() + 35_000 }] });
      render(<LanPairingPanel />);
      await act(async () => { await Promise.resolve(); });
      fireEvent.click(screen.getByRole("button", { name: "核对并配对 MS-LGKRSZGOVODD" }));
      const staleConfirm = within(screen.getByRole("alertdialog")).getByRole("button", { name: "确认配对，仅屏幕查看" });
      await act(async () => { await vi.advanceTimersByTimeAsync(2_000); });
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
      expect(screen.getByRole("alert")).toHaveTextContent("重新核对");
      fireEvent.click(staleConfirm);
      expect(commands.ipcApproveLanPairing).not.toHaveBeenCalled();
    } finally { vi.useRealTimers(); }
  });

  it("does not carry a confirmation across a lease gap even when the next selector and binding match", async () => {
    vi.useFakeTimers();
    try {
      const peer = candidate({ expires_at_ms: Date.now() + 1_000 });
      commands.ipcListLanPairingCandidates.mockResolvedValueOnce({ ok: true, value: [peer] })
        .mockImplementation(async () => ({ ok: true, value: [{ ...peer, expires_at_ms: Date.now() + 10_000 }] }));
      render(<LanPairingPanel />);
      await act(async () => { await Promise.resolve(); });
      fireEvent.click(screen.getByRole("button", { name: "核对并配对 MS-LGKRSZGOVODD" }));
      await act(async () => { await vi.advanceTimersByTimeAsync(2_000); });
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
      expect(screen.getByRole("alert")).toHaveTextContent("重新核对");
      expect(commands.ipcApproveLanPairing).not.toHaveBeenCalled();
    } finally { vi.useRealTimers(); }
  });

  it("invalidates the selected confirmation when the candidate disappears", async () => {
    vi.useFakeTimers();
    try {
      commands.ipcListLanPairingCandidates.mockResolvedValueOnce({ ok: true, value: [candidate()] })
        .mockResolvedValue({ ok: true, value: [] });
      render(<LanPairingPanel />);
      await act(async () => { await Promise.resolve(); });
      fireEvent.click(screen.getByRole("button", { name: "核对并配对 MS-LGKRSZGOVODD" }));
      await act(async () => { await vi.advanceTimersByTimeAsync(2_000); });
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
      expect(screen.getByRole("alert")).toHaveTextContent("重新核对");
      expect(commands.ipcApproveLanPairing).not.toHaveBeenCalled();
    } finally { vi.useRealTimers(); }
  });

  it("coalesces periodic reads and blocks confirmation while a refresh is still in flight", async () => {
    vi.useFakeTimers();
    try {
      const peer = candidate();
      let complete!: (value: unknown) => void;
      commands.ipcListLanPairingCandidates.mockResolvedValueOnce({ ok: true, value: [peer] })
        .mockReturnValue(new Promise(resolve => { complete = resolve; }));
      render(<LanPairingPanel />);
      await act(async () => { await Promise.resolve(); });
      fireEvent.click(screen.getByRole("button", { name: "核对并配对 MS-LGKRSZGOVODD" }));
      await act(async () => { await vi.advanceTimersByTimeAsync(6_000); });
      expect(commands.ipcListLanPairingCandidates).toHaveBeenCalledTimes(2);
      const confirm = within(screen.getByRole("alertdialog")).getByRole("button", { name: "确认配对，仅屏幕查看" });
      expect(confirm).toBeDisabled();
      fireEvent.click(confirm);
      expect(commands.ipcApproveLanPairing).not.toHaveBeenCalled();
      await act(async () => { complete({ ok: true, value: [{ ...peer, expires_at_ms: Date.now() + 30_000 }] }); });
      expect(confirm).toBeEnabled();
    } finally { vi.useRealTimers(); }
  });

  it("fails closed if a candidate refresh errors while a confirmation is open", async () => {
    vi.useFakeTimers();
    try {
      commands.ipcListLanPairingCandidates.mockResolvedValueOnce({ ok: true, value: [candidate()] })
        .mockResolvedValue({ ok: false, error: { message: "候选读取失败" } });
      render(<LanPairingPanel />);
      await act(async () => { await Promise.resolve(); });
      fireEvent.click(screen.getByRole("button", { name: "核对并配对 MS-LGKRSZGOVODD" }));
      await act(async () => { await vi.advanceTimersByTimeAsync(2_000); });
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
      expect(screen.getByRole("alert")).toHaveTextContent("候选读取失败");
      expect(commands.ipcApproveLanPairing).not.toHaveBeenCalled();
    } finally { vi.useRealTimers(); }
  });
});
