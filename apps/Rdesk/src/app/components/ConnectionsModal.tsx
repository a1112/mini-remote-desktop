import { useEffect } from "react";
import { X } from "lucide-react";
import { useTheme } from "./ThemeContext";
import { ConnectionsPage } from "./ConnectionsPage";

export function ConnectionsModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { isDark } = useTheme();
  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);
  if (!open) return null;
  return <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-6" role="dialog" aria-label="连接记录" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
    <div className={`relative w-[820px] max-w-full max-h-[calc(100vh-80px)] overflow-y-auto rounded-xl shadow-2xl border ${isDark ? "bg-[#1e1e1e] border-gray-700" : "bg-white border-gray-200"}`}>
      <button onClick={onClose} aria-label="关闭连接记录" className="absolute top-5 right-5 z-10 p-1 rounded hover:bg-gray-400/20"><X className="w-4 h-4" /></button>
      <ConnectionsPage />
    </div>
  </div>;
}
