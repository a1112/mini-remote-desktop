import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { MatrixTestPage } from "./MatrixTestPage";

function selectSingleSupportedCombination() {
  fireEvent.click(screen.getByLabelText("OpenH264"));
  fireEvent.click(screen.getByLabelText("软件"));
}

function resultRow() {
  return screen.getAllByRole("row")[1]!;
}

describe("MatrixTestPage failure handling", () => {
  it("marks a row failed when test_start_run rejects", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_start_run") {
        return Promise.reject(new Error("unsupported scenario"));
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage />);
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(within(resultRow()).getByText("失败")).toBeInTheDocument();
    });
  });

  it("marks a row failed and stops the run when test_get_run rejects", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.reject(new Error("run missing"));
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(within(resultRow()).getByText("失败")).toBeInTheDocument();
    });
    expect(mockInvoke).toHaveBeenCalledWith("test_stop_run", { runId: "run-1" });
  });

  it("marks a row failed and stops the run when test_get_run returns null", async () => {
    const mockInvoke = getMockInvoke();
    mockInvoke.mockImplementation((command: string) => {
      if (command === "test_start_run") {
        return Promise.resolve("run-1");
      }
      if (command === "test_get_run") {
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    });

    render(<MatrixTestPage runDelayMs={0} />);
    selectSingleSupportedCombination();
    fireEvent.click(screen.getByRole("button", { name: /启动矩阵测试/ }));

    await waitFor(() => {
      expect(within(resultRow()).getByText("失败")).toBeInTheDocument();
    });
    expect(mockInvoke).toHaveBeenCalledWith("test_stop_run", { runId: "run-1" });
  });
});
