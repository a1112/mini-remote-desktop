import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "@/test/mocks/tauri";
import { TransferModal } from "./FileTransferPage";

describe("TransferModal", () => {
  it("renders the reserved service provider snapshot instead of demo transfer tasks", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockResolvedValue({
      provider: {
        provider_id: "mrd.file_transfer.reserved",
        display_name: "Reserved file transfer provider",
        status: "reserved",
        detail: "Reserved for MRD/R-File provider binding.",
        capabilities: [
          "file.transfer.snapshot",
          "file.transfer.external_provider",
          "file.transfer.rfile.quic_stream",
          "file.transfer.rfile.http_client_stats",
          "file.transfer.rfile.remote_mount",
          "file.transfer.perf_baseline",
        ],
        supported_actions: ["list", "compare_provider", "bind_external_provider"],
      },
      tasks: [],
      updated_at_ms: null,
    });

    render(<TransferModal open onClose={vi.fn()} />);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("ipc_file_transfer_snapshot", undefined);
    });
    expect(screen.getByText("Provider 已预留")).toBeInTheDocument();
    expect(screen.getByText("Reserved file transfer provider")).toBeInTheDocument();
    expect(screen.getByText("file.transfer.rfile.quic_stream")).toBeInTheDocument();
    expect(screen.getByText("compare_provider")).toBeInTheDocument();
    expect(screen.queryByText("project-backup.zip")).not.toBeInTheDocument();
  });

  it("routes transfer item pause, resume, and cancel actions through mrd-service", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockResolvedValueOnce({
      provider: {
        provider_id: "mrd.file_transfer.rfile",
        display_name: "R-File provider boundary",
        status: "available",
        detail: "Detected R-File provider root.",
        capabilities: [
          "file.transfer.rfile.transfer_tasks",
          "file.transfer.rfile.http.download_stream_1mb_buffer",
          "file.transfer.rfile.quic.transfer_16gb_limit",
          "file.transfer.rfile.endpoint.http://127.0.0.1:18080",
        ],
        supported_actions: ["list", "pause", "resume", "cancel"],
      },
      tasks: [
        {
          transfer_id: "transfer-active",
          direction: "send",
          status: "running",
          source_device_id: "local",
          target_device_id: "remote",
          source_paths: ["C:\\Users\\Admin\\active.bin"],
          target_path: "D:\\Inbox",
          total_bytes: 1024,
          transferred_bytes: 128,
        },
        {
          transfer_id: "transfer-paused",
          direction: "receive",
          status: "paused",
          source_device_id: "remote",
          target_device_id: "local",
          source_paths: ["D:\\paused.bin"],
          target_path: "C:\\Inbox",
          total_bytes: 2048,
          transferred_bytes: 256,
        },
      ],
      updated_at_ms: null,
    });
    mockInvoke.mockResolvedValue({
      accepted: false,
      supported: false,
      message: "File transfer provider has no active runtime task binding yet.",
    });
    const user = userEvent.setup();

    render(<TransferModal open onClose={vi.fn()} />);

    await screen.findByText("active.bin");
    await user.click(screen.getByTitle("暂停"));
    await user.click(screen.getByTitle("继续"));
    const [cancelActiveTransfer] = screen.getAllByTitle("取消传输");
    if (!cancelActiveTransfer) {
      throw new Error("Expected an active transfer cancel button");
    }
    await user.click(cancelActiveTransfer);

    expect(mockInvoke).toHaveBeenCalledWith("ipc_request_file_transfer_action", {
      transferId: "transfer-active",
      action: "pause",
    });
    expect(mockInvoke).toHaveBeenCalledWith("ipc_request_file_transfer_action", {
      transferId: "transfer-paused",
      action: "resume",
    });
    expect(mockInvoke).toHaveBeenCalledWith("ipc_request_file_transfer_action", {
      transferId: "transfer-active",
      action: "cancel",
    });
    expect(screen.getByText("file.transfer.rfile.http.download_stream_1mb_buffer")).toBeInTheDocument();
    expect(screen.getByText("file.transfer.rfile.quic.transfer_16gb_limit")).toBeInTheDocument();
    expect(screen.getByText("file.transfer.rfile.endpoint.http://127.0.0.1:18080")).toBeInTheDocument();
  });

  it("routes cancel all through one service request per visible active transfer", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockResolvedValueOnce({
      provider: {
        provider_id: "mrd.file_transfer.rfile",
        display_name: "R-File provider boundary",
        status: "available",
        detail: "Detected R-File provider root.",
        capabilities: ["file.transfer.rfile.transfer_tasks"],
        supported_actions: ["list", "pause", "resume", "cancel"],
      },
      tasks: [
        {
          transfer_id: "transfer-active",
          direction: "send",
          status: "running",
          source_device_id: "local",
          target_device_id: "remote",
          source_paths: ["C:\\Users\\Admin\\active.bin"],
          target_path: "D:\\Inbox",
          total_bytes: 1024,
          transferred_bytes: 128,
        },
        {
          transfer_id: "transfer-paused",
          direction: "receive",
          status: "paused",
          source_device_id: "remote",
          target_device_id: "local",
          source_paths: ["D:\\paused.bin"],
          target_path: "C:\\Inbox",
          total_bytes: 2048,
          transferred_bytes: 256,
        },
        {
          transfer_id: "transfer-complete",
          direction: "receive",
          status: "completed",
          source_device_id: "remote",
          target_device_id: "local",
          source_paths: ["D:\\done.bin"],
          target_path: "C:\\Inbox",
          total_bytes: 4096,
          transferred_bytes: 4096,
        },
      ],
      updated_at_ms: null,
    });
    mockInvoke.mockResolvedValue({
      accepted: false,
      supported: false,
      message: "File transfer provider has no active runtime task binding yet.",
    });
    const user = userEvent.setup();

    render(<TransferModal open onClose={vi.fn()} />);

    await screen.findByText("active.bin");
    await user.click(screen.getByText("全部取消"));

    expect(mockInvoke).toHaveBeenCalledWith("ipc_request_file_transfer_action", {
      transferId: "transfer-active",
      action: "cancel",
    });
    expect(mockInvoke).toHaveBeenCalledWith("ipc_request_file_transfer_action", {
      transferId: "transfer-paused",
      action: "cancel",
    });
    expect(mockInvoke).not.toHaveBeenCalledWith("ipc_request_file_transfer_action", {
      transferId: "transfer-complete",
      action: "cancel",
    });
  });
});
