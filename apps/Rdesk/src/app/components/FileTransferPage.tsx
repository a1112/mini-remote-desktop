import { useState } from "react";
import { ArrowRightLeft, RefreshCw, Search, X } from "lucide-react";
import { ipcCancelFileTransfer } from "../adapters/tauri/commands";
import type { FileTransferTaskSnapshot } from "../adapters/tauri/types";
import { useTheme } from "./ThemeContext";
import { useFileTransfers, transferName, transferProgress, formatTransferBytes, transferStatusLabel } from "../services/fileTransferListService";

function TransferRow({ transfer, refresh, isDark }: {
  transfer: FileTransferTaskSnapshot; refresh: () => Promise<void>; isDark: boolean;
}) {
  const progress = transferProgress(transfer);
  const canCancel = transfer.status === "running" || transfer.status === "queued";
  const direction = transfer.source_device_id && transfer.target_device_id
    ? `${transfer.source_device_id} → ${transfer.target_device_id}`
    : transfer.source_device_id ? `来自 ${transfer.source_device_id}`
      : transfer.target_device_id ? `发往 ${transfer.target_device_id}` : "本机传输";
  return <div className={`rounded-lg border p-3 ${isDark ? "bg-[#252525] border-gray-700" : "bg-white border-gray-200"}`}>
    <div className="flex items-center gap-3">
      <ArrowRightLeft className="w-4 h-4 text-blue-500 shrink-0" />
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium truncate">{transferName(transfer)}</div>
        <div className="text-xs opacity-60 truncate">{direction}</div>
      </div>
      <span className="text-xs">{transferStatusLabel[transfer.status]}</span>
      {canCancel && <button title="取消传输" onClick={async () => { await ipcCancelFileTransfer(transfer.transfer_id); await refresh(); }} className="p-1 rounded hover:bg-red-500/10 text-red-500"><X className="w-4 h-4" /></button>}
    </div>
    <div className="mt-2 text-xs opacity-70">{formatTransferBytes(transfer.copied_bytes)}{transfer.total_bytes != null ? ` / ${formatTransferBytes(transfer.total_bytes)}` : ""} · {transfer.copied_entries} / {transfer.total_entries} 项</div>
    {progress !== null && <div className="flex items-center gap-2 mt-2"><div className="flex-1 h-1.5 rounded bg-gray-500/20"><div className="h-full rounded bg-blue-500" style={{ width: `${progress}%` }} /></div><span className="text-xs opacity-70">{progress}%</span></div>}
    {transfer.error && <p className="mt-2 text-xs text-red-500">{transfer.error}</p>}
  </div>;
}

export function TransferModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { isDark } = useTheme();
  const { transfers, loading, error, refresh } = useFileTransfers(open);
  const [search, setSearch] = useState("");
  if (!open) return null;
  const filtered = transfers.filter((transfer) =>
    !search || transferName(transfer).toLowerCase().includes(search.toLowerCase())
      || transfer.source_device_id?.toLowerCase().includes(search.toLowerCase())
      || transfer.target_device_id?.toLowerCase().includes(search.toLowerCase()));
  const active = transfers.filter((transfer) => transfer.status === "running" || transfer.status === "queued").length;
  return <div className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-6" role="dialog" aria-label="传输管理" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
    <div className={`w-[680px] max-w-full max-h-[80vh] rounded-xl border shadow-2xl flex flex-col ${isDark ? "bg-[#1e1e1e] border-gray-700 text-gray-100" : "bg-white border-gray-200 text-gray-900"}`}>
      <div className="flex items-center gap-3 px-4 py-3 border-b border-gray-400/20">
        <h2 className="font-medium flex-1">传输管理 <span className="text-xs opacity-60">{active} 进行中</span></h2>
        <button onClick={() => void refresh()} title="刷新传输"><RefreshCw className="w-4 h-4" /></button>
        <button onClick={onClose} title="关闭"><X className="w-4 h-4" /></button>
      </div>
      <label className="flex items-center gap-2 mx-4 mt-3 px-2 border rounded border-gray-400/30"><Search className="w-4 h-4 opacity-50" /><input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="搜索传输" className="bg-transparent outline-none py-1.5 text-sm flex-1" /></label>
      <div className="overflow-y-auto p-4 space-y-2">
        {loading && <p className="text-sm opacity-60 text-center py-10">正在读取传输任务...</p>}
        {!loading && error && <p role="alert" className="text-sm text-red-500 text-center py-10">读取传输任务失败：{error}</p>}
        {!loading && !error && filtered.length === 0 && <p className="text-sm opacity-60 text-center py-10">暂无真实传输任务</p>}
        {filtered.map((transfer) => <TransferRow key={transfer.transfer_id} transfer={transfer} refresh={refresh} isDark={isDark} />)}
      </div>
    </div>
  </div>;
}
