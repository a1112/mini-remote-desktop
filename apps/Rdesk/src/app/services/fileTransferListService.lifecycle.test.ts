import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AdapterResult, FileTransferTaskSnapshot } from "../adapters/tauri/types";

vi.mock("../adapters/tauri/commands", () => ({ ipcListFileTransfers: vi.fn() }));

import { ipcListFileTransfers } from "../adapters/tauri/commands";
import { useFileTransfers } from "./fileTransferListService";

type Result = AdapterResult<FileTransferTaskSnapshot[]>;

function deferred() {
  let resolve!: (result: Result) => void;
  const promise = new Promise<Result>((done) => { resolve = done; });
  return { promise, resolve };
}

function success(id = "current"): Result {
  return {
    ok: true,
    value: [{
      transfer_id: id,
      status: "running",
      transport_kind: "synthetic",
      total_entries: 1,
      copied_entries: 0,
      total_bytes: 128,
      copied_bytes: 0,
      entries: [],
    }],
  };
}

async function flush() {
  await act(async () => { await Promise.resolve(); });
}

async function advance(ms: number) {
  await act(async () => { await vi.advanceTimersByTimeAsync(ms); });
}

describe("useFileTransfers request ownership", () => {
  let requests: ReturnType<typeof deferred>[];

  beforeEach(() => {
    vi.useFakeTimers();
    requests = [];
    vi.mocked(ipcListFileTransfers).mockReset().mockImplementation(() => {
      const request = deferred();
      requests.push(request);
      return request.promise;
    });
  });

  afterEach(() => {
    cleanup();
    vi.clearAllTimers();
    vi.useRealTimers();
  });

  async function settle(index: number, value: Result = success()) {
    const request = requests[index];
    expect(request).toBeDefined();
    await act(async () => { request!.resolve(value); });
  }

  it("publishes a current result and supports a fresh manual read while idle", async () => {
    const { result } = renderHook(() => useFileTransfers());
    await flush();
    await settle(0, success("initial"));
    let manual!: Promise<void>;
    act(() => { manual = result.current.refresh(); });
    await flush();
    expect(requests).toHaveLength(2);
    await settle(1, success("manual"));
    await manual;
    expect(result.current.transfers[0]?.transfer_id).toBe("manual");
    expect(result.current.error).toBe(null);
    expect(result.current.loading).toBe(false);
  });

  it("keeps thirty seconds of slow polling to one in-flight IPC request", async () => {
    renderHook(() => useFileTransfers());
    await flush();
    await advance(30_000);
    expect(ipcListFileTransfers).toHaveBeenCalledTimes(1);
    expect(requests).toHaveLength(1);
  });

  it("starts the next poll three seconds after settlement", async () => {
    const { result } = renderHook(() => useFileTransfers());
    await flush();
    await advance(1_000);
    await settle(0);
    expect(result.current.loading).toBe(false);
    expect(result.current.transfers[0]?.transfer_id).toBe("current");
    await advance(2_999);
    expect(requests).toHaveLength(1);
    await advance(1);
    expect(requests).toHaveLength(2);
  });

  it("coalesces manual refreshes into one fresh read after the pending read", async () => {
    const { result } = renderHook(() => useFileTransfers());
    await flush();
    let first!: Promise<void>;
    let second!: Promise<void>;
    act(() => {
      first = result.current.refresh();
      second = result.current.refresh();
    });
    await flush();
    expect(requests).toHaveLength(1);
    await settle(0, success("before-cancel"));
    expect(requests).toHaveLength(2);
    await settle(1, success("after-cancel"));
    await Promise.all([first, second]);
    expect(result.current.transfers[0]?.transfer_id).toBe("after-cancel");
    expect(requests).toHaveLength(2);
  });

  it("honors a fresh read requested as the previous IPC promise settles", async () => {
    const { result } = renderHook(() => useFileTransfers());
    await flush();
    const manual = requests[0]!.promise.then(() => result.current.refresh());
    await settle(0, success("before-cancel"));
    expect(requests).toHaveLength(2);
    await settle(1, success("after-cancel"));
    await manual;
    expect(result.current.transfers[0]?.transfer_id).toBe("after-cancel");
  });

  it("does not poll or manually query while disabled", async () => {
    const { result } = renderHook(() => useFileTransfers(false));
    await advance(30_000);
    let manual!: Promise<void>;
    act(() => { manual = result.current.refresh(); });
    await flush();
    await act(async () => {
      for (const request of requests) request.resolve(success());
      await manual;
    });
    expect(requests).toHaveLength(0);
    expect(result.current.loading).toBe(false);
  });

  it("discards a late result after the list is disabled", async () => {
    const { result, rerender } = renderHook(({ enabled }) => useFileTransfers(enabled), {
      initialProps: { enabled: true },
    });
    await flush();
    rerender({ enabled: false });
    await settle(0, success("retired"));
    expect(result.current.transfers).toEqual([]);
    await advance(30_000);
    expect(requests).toHaveLength(1);
  });

  it("waits for a retired read before starting the re-enabled generation", async () => {
    const { result, rerender } = renderHook(({ enabled }) => useFileTransfers(enabled), {
      initialProps: { enabled: true },
    });
    await flush();
    rerender({ enabled: false });
    rerender({ enabled: true });
    await flush();
    expect(requests).toHaveLength(1);
    await settle(0, success("retired"));
    expect(result.current.transfers).toEqual([]);
    expect(requests).toHaveLength(2);
    await settle(1, success("new-generation"));
    expect(result.current.transfers[0]?.transfer_id).toBe("new-generation");
  });

  it("discards a retired error and allows the next generation to succeed", async () => {
    const { result, rerender } = renderHook(({ enabled }) => useFileTransfers(enabled), {
      initialProps: { enabled: true },
    });
    await flush();
    rerender({ enabled: false });
    rerender({ enabled: true });
    await settle(0, { ok: false, error: { message: "retired failure" } });
    expect(result.current.error).toBe(null);
    await settle(1, success("new-generation"));
    expect(result.current.transfers[0]?.transfer_id).toBe("new-generation");
  });

  it("publishes an adapter error and retries at the settlement-based cadence", async () => {
    const { result } = renderHook(() => useFileTransfers());
    await flush();
    await advance(1_000);
    await settle(0, { ok: false, error: { message: "synthetic IPC failure" } });
    expect(result.current.error).toBe("synthetic IPC failure");
    expect(result.current.loading).toBe(false);
    await advance(2_999);
    expect(requests).toHaveLength(1);
    await advance(1);
    await settle(1);
    expect(result.current.error).toBe(null);
  });

  it("handles a rejected manual IPC read without an unhandled rejection", async () => {
    const { result } = renderHook(() => useFileTransfers());
    await flush();
    await settle(0);
    vi.mocked(ipcListFileTransfers).mockRejectedValueOnce(new Error("synthetic rejection"));
    await act(async () => { await result.current.refresh(); });
    expect(result.current.error).toBe("synthetic rejection");
    expect(result.current.transfers).toEqual([]);
    expect(result.current.loading).toBe(false);
  });

  it("keeps StrictMode effect replay to one physical read", async () => {
    renderHook(() => useFileTransfers(), { reactStrictMode: true });
    await flush();
    expect(requests).toHaveLength(1);
  });

  it("does not restart polling or run a saved refresh after unmount", async () => {
    const { result, unmount } = renderHook(() => useFileTransfers());
    await flush();
    const retiredRefresh = result.current.refresh;
    unmount();
    let manual!: Promise<void>;
    act(() => { manual = retiredRefresh(); });
    await flush();
    await act(async () => {
      for (const request of requests) request.resolve(success());
      await manual;
    });
    await advance(30_000);
    expect(requests).toHaveLength(1);
  });

  it("drops a queued manual refresh when the list is disabled", async () => {
    const { result, rerender } = renderHook(({ enabled }) => useFileTransfers(enabled), {
      initialProps: { enabled: true },
    });
    await flush();
    act(() => { void result.current.refresh(); });
    rerender({ enabled: false });
    await settle(0);
    await advance(30_000);
    expect(requests).toHaveLength(1);
  });
});
