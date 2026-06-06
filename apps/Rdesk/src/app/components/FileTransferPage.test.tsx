import { render, screen, waitFor } from "@testing-library/react";
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
        capabilities: ["file.transfer.snapshot", "file.transfer.external_provider"],
        supported_actions: ["list"],
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
    expect(screen.queryByText("project-backup.zip")).not.toBeInTheDocument();
  });
});
