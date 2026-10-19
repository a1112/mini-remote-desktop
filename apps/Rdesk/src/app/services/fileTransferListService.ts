import { useCallback, useEffect, useRef, useState } from "react";
import { ipcListFileTransfers } from "../adapters/tauri/commands";
import type { FileTransferTaskSnapshot } from "../adapters/tauri/types";

export function useFileTransfers(enabled = true) {
  const [transfers, setTransfers] = useState<FileTransferTaskSnapshot[]>([]);
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<string | null>(null);
  const lifecycle = useRef<{
    enabled: boolean;
    generation: number;
    inFlight: Promise<void> | null;
    refreshQueued: boolean;
    timer: ReturnType<typeof setTimeout> | undefined;
  }>({
    enabled: false,
    generation: 0,
    inFlight: null,
    refreshQueued: false,
    timer: undefined,
  });

  const poll = useCallback(function request(fresh = false): Promise<void> {
    const current = lifecycle.current;
    if (!current.enabled) return Promise.resolve();
    if (current.timer !== undefined) {
      clearTimeout(current.timer);
      current.timer = undefined;
    }
    if (current.inFlight) {
      if (fresh) current.refreshQueued = true;
      return current.inFlight;
    }

    const operation = Promise.resolve().then(async () => {
      while (current.enabled) {
        current.refreshQueued = false;
        const generation = current.generation;
        try {
          const result = await ipcListFileTransfers();
          if (current.enabled && generation === current.generation) {
            setTransfers(result.ok ? result.value : []);
            setError(result.ok ? null : result.error.message);
            setLoading(false);
          }
        } catch (failure) {
          if (current.enabled && generation === current.generation) {
            setTransfers([]);
            setError(failure instanceof Error ? failure.message : String(failure));
            setLoading(false);
          }
        }
        if (!current.refreshQueued) break;
      }
    }).finally(() => {
      if (current.inFlight !== operation) return;
      current.inFlight = null;
      // A manual refresh may arrive after the loop exits, before this handoff.
      if (current.enabled && current.refreshQueued) return request();
      if (current.enabled) {
        current.timer = setTimeout(() => {
          current.timer = undefined;
          void request();
        }, 3000);
      }
    });
    current.inFlight = operation;
    return operation;
  }, []);

  const refresh = useCallback(() => poll(true), [poll]);
  useEffect(() => {
    const current = lifecycle.current;
    current.generation += 1;
    current.enabled = enabled;
    if (!enabled) return;
    setLoading(true);
    void refresh();
    return () => {
      current.enabled = false;
      current.generation += 1;
      current.refreshQueued = false;
      if (current.timer !== undefined) {
        clearTimeout(current.timer);
        current.timer = undefined;
      }
    };
  }, [enabled, refresh]);
  return { transfers, loading, error, refresh };
}

export function transferName(transfer: FileTransferTaskSnapshot): string {
  if (transfer.entries.length === 1) return transfer.entries[0]?.name || "文件传输";
  return `${transfer.total_entries} 个文件`;
}

export function transferProgress(transfer: FileTransferTaskSnapshot): number | null {
  if (transfer.total_bytes && transfer.total_bytes > 0) {
    return Math.min(100, Math.floor(100 * transfer.copied_bytes / transfer.total_bytes));
  }
  if (transfer.total_entries > 0) {
    return Math.min(100, Math.floor(100 * transfer.copied_entries / transfer.total_entries));
  }
  return null;
}

export function formatTransferBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

export const transferStatusLabel: Record<FileTransferTaskSnapshot["status"], string> = {
  queued: "排队中", running: "传输中", completed: "已完成", failed: "失败", cancelled: "已取消",
};
