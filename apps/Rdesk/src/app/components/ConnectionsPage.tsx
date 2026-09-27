import { useState } from "react";
import { Calendar, Download, History, Search, Trash2 } from "lucide-react";
import { useTheme } from "./ThemeContext";
import { useDevices } from "./deviceData";
import { clearConnectionHistory, useConnectionHistory } from "../services/connectionHistoryService";

const dateLabel = (value: number) => new Date(value).toLocaleDateString("zh-CN");
const timeLabel = (value: number) => new Date(value).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
const durationLabel = (value: number) => {
  const minutes = Math.max(0, Math.floor(value / 60_000));
  return minutes >= 60 ? `${Math.floor(minutes / 60)}小时${minutes % 60}分` : `${minutes}分钟`;
};

export function ConnectionsPage() {
  const { isDark } = useTheme();
  const history = useConnectionHistory();
  const { devices } = useDevices();
  const [search, setSearch] = useState("");
  const [range, setRange] = useState("全部");
  const [direction, setDirection] = useState("全部");
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const week = today - ((now.getDay() + 6) % 7) * 86_400_000;
  const month = new Date(now.getFullYear(), now.getMonth(), 1).getTime();
  const deviceName = (id: string) => devices.find((device) => device.deviceId === id)?.name;
  const filtered = history.filter((entry) =>
    (!search || entry.peerDeviceId.toLowerCase().includes(search.toLowerCase()) || deviceName(entry.peerDeviceId)?.toLowerCase().includes(search.toLowerCase()))
    && (range === "全部" || entry.startedAt >= (range === "今天" ? today : range === "本周" ? week : month))
    && (direction === "全部" || entry.role === (direction === "主动连接" ? "controller" : "agent"))
  );
  const grouped = filtered.reduce<Record<string, typeof filtered>>((groups, entry) => {
    (groups[dateLabel(entry.startedAt)] ??= []).push(entry);
    return groups;
  }, {});
  const monthly = history.filter((entry) => entry.startedAt >= month);
  const monthlyDuration = monthly.reduce((sum, entry) => sum + (entry.endedAt ? Math.max(0, entry.endedAt - entry.startedAt) : 0), 0);
  const exportHistory = () => {
    const csv = ["session_id,device_id,direction,started_at,ended_at", ...history.map((entry) =>
      [entry.sessionId, entry.peerDeviceId, entry.role, new Date(entry.startedAt).toISOString(),
        entry.endedAt ? new Date(entry.endedAt).toISOString() : ""].map((field) => `"${field.replace(/"/g, '""')}"`).join(","))].join("\r\n");
    const url = URL.createObjectURL(new Blob(["\uFEFF", csv], { type: "text/csv;charset=utf-8" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = "rdesk-connections.csv";
    link.click();
    URL.revokeObjectURL(url);
  };
  return <div className={`p-6 ${isDark ? "text-gray-100" : "text-gray-900"}`}>
    <div className="flex items-start justify-between gap-4 mb-5">
      <div><h1 className="flex items-center gap-2 font-semibold text-lg"><History className="w-5 h-5 text-blue-600" />连接记录</h1>
        <p className="text-sm opacity-60 mt-1">本机记录的真实远程会话；进入画面后才记录</p></div>
      <div className="flex gap-2">
        <button onClick={exportHistory} disabled={!history.length} className="px-3 py-1.5 rounded border border-gray-400/30 text-sm disabled:opacity-40 flex items-center gap-1"><Download className="w-4 h-4" />导出</button>
        <button onClick={() => { if (window.confirm("清空本机连接记录？")) clearConnectionHistory(); }} disabled={!history.length} className="px-3 py-1.5 rounded border border-gray-400/30 text-sm disabled:opacity-40 flex items-center gap-1"><Trash2 className="w-4 h-4" />清空</button>
      </div>
    </div>
    <div className="grid grid-cols-2 gap-3 mb-5">
      <div className={`rounded-lg p-3 ${isDark ? "bg-[#2a2a2a]" : "bg-gray-50"}`}><div className="text-xl font-semibold text-blue-600">{monthly.length}</div><div className="text-xs opacity-60">本月连接次数</div></div>
      <div className={`rounded-lg p-3 ${isDark ? "bg-[#2a2a2a]" : "bg-gray-50"}`}><div className="text-xl font-semibold text-blue-600">{durationLabel(monthlyDuration)}</div><div className="text-xs opacity-60">本月已记录时长</div></div>
    </div>
    <div className="flex flex-wrap gap-2 mb-4">
      <label className="flex items-center gap-2 rounded border border-gray-400/30 px-2"><Search className="w-4 h-4 opacity-50" /><input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="搜索设备" className="bg-transparent outline-none py-1.5 text-sm" /></label>
      <select aria-label="时间范围" value={range} onChange={(event) => setRange(event.target.value)} className={`rounded border border-gray-400/30 px-2 text-sm ${isDark ? "bg-[#2a2a2a]" : "bg-white"}`}><option>全部</option><option>今天</option><option>本周</option><option>本月</option></select>
      <select aria-label="连接方向" value={direction} onChange={(event) => setDirection(event.target.value)} className={`rounded border border-gray-400/30 px-2 text-sm ${isDark ? "bg-[#2a2a2a]" : "bg-white"}`}><option>全部</option><option>主动连接</option><option>被动接入</option></select>
      <span className="text-xs opacity-60 self-center ml-auto">共 {filtered.length} 条</span>
    </div>
    {!filtered.length && <div className="py-14 text-center text-sm opacity-60">{history.length ? "没有符合条件的记录" : "暂无真实连接记录"}</div>}
    <div className="space-y-5">{Object.entries(grouped).map(([date, entries]) => <section key={date}>
      <h2 className="flex items-center gap-2 text-xs opacity-60 mb-2"><Calendar className="w-3.5 h-3.5" />{date} · {entries.length} 次</h2>
      <div className="space-y-2">{entries.map((entry) => <div key={entry.sessionId} className={`rounded-lg border border-gray-400/20 p-3 flex items-center gap-3 ${isDark ? "bg-[#252525]" : "bg-white"}`}>
        <div className="min-w-0 flex-1"><div className="font-medium text-sm truncate">{deviceName(entry.peerDeviceId) || entry.peerDeviceId}</div><div className="text-xs opacity-60 font-mono truncate">{entry.peerDeviceId}</div></div>
        <div className="text-xs text-right opacity-70"><div>{entry.role === "controller" ? "主动连接" : "被动接入"}</div><div>{timeLabel(entry.startedAt)}{entry.endedAt ? ` – ${timeLabel(entry.endedAt)}` : ""}</div>{entry.endedAt && <div>{durationLabel(entry.endedAt - entry.startedAt)}</div>}</div>
      </div>)}</div>
    </section>)}</div>
  </div>;
}
