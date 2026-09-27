import { useCallback, useEffect, useState } from "react";
import { ipcListFileTransfers } from "../adapters/tauri/commands";
import type { FileTransferTaskSnapshot } from "../adapters/tauri/types";

export function useFileTransfers(enabled = true) {
  const [transfers, setTransfers] = useState<FileTransferTaskSnapshot[]>([]);
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<string | null>(null);
  const refresh = useCallback(async () => {
    const result = await ipcListFileTransfers();
    if (result.ok) {
      setTransfers(result.value);
      setError(null);
    } else {
      setTransfers([]);
      setError(result.error.message);
    }
    setLoading(false);
  }, []);
  useEffect(() => {
    if (!enabled) return;
    setLoading(true);
    void refresh();
    const timer = window.setInterval(() => void refresh(), 3000);
    return () => window.clearInterval(timer);
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
