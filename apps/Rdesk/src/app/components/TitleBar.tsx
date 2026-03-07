import { useState, useRef, useEffect } from "react";
import {
  Minus,
  Square,
  X,
  Search,
  Bell,
  Signal,
  Lock,
  History,
  Settings,
  PanelLeftClose,
  PanelLeftOpen,
  ArrowRightLeft,
  ChevronDown,
  ArrowLeft,
  Monitor,
  Smartphone,
  Pause,
  CheckCircle2,
  ArrowUpFromLine,
  ArrowDownToLine,
  ShieldAlert,
  Wifi,
  WifiOff,
  FolderCheck,
  Download,
  AlertTriangle,
  Info,
  UserPlus,
  Check,
  User,
  LogOut,
  Mail,
  KeyRound,
  Shield,
  Edit,
} from "lucide-react";
import { useTheme } from "./ThemeContext";
import { useDetailBar } from "./DetailBarContext";
import { withTauriWindow } from "../utils/tauriWindow";
import { deviceService } from "../services/deviceService";

interface TitleBarProps {
  onOpenConnections?: () => void;
  onOpenSettings?: () => void;
  onOpenTransfers?: () => void;
  onOpenAuth?: () => void;
  collapsed?: boolean;
  onToggleSidebar?: () => void;
}

export function TitleBar({ onOpenConnections, onOpenSettings, onOpenTransfers, onOpenAuth, collapsed, onToggleSidebar }: TitleBarProps) {
  const [isMaximized, setIsMaximized] = useState(false);
  const [searchFocused, setSearchFocused] = useState(false);
  const [hasNotification] = useState(true);
  const [transferOpen, setTransferOpen] = useState(false);
  const [notifOpen, setNotifOpen] = useState(false);
  const [userMenuOpen, setUserMenuOpen] = useState(false);
  const [showProfileModal, setShowProfileModal] = useState(false);
  const [notifTab, setNotifTab] = useState<"all" | "unread" | "system" | "device">("all");
  const [readIds, setReadIds] = useState<Set<string>>(new Set());
  const [userInitial, setUserInitial] = useState("U");
  const [userLabel, setUserLabel] = useState("未登录");
  const [userAvatarUrl, setUserAvatarUrl] = useState<string | null>(null);
  const [isLoggedIn, setIsLoggedIn] = useState(false);
  const [userData, setUserData] = useState<{ username: string; email: string; role: string; avatar_url?: string } | null>(null);
  const transferRef = useRef<HTMLDivElement>(null);
  const notifRef = useRef<HTMLDivElement>(null);
  const userMenuRef = useRef<HTMLDivElement>(null);
  const { isDark } = useTheme();
  const detailBar = useDetailBar();

  const noDragSelector =
    'button, a, input, select, textarea, [role="button"], [role="menuitem"], [role="menuitemcheckbox"], [role="menuitemradio"], [data-radix-collection-item], [data-no-drag="true"]';

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (transferRef.current && !transferRef.current.contains(e.target as Node)) {
        setTransferOpen(false);
      }
      if (notifRef.current && !notifRef.current.contains(e.target as Node)) {
        setNotifOpen(false);
      }
      if (userMenuRef.current && !userMenuRef.current.contains(e.target as Node)) {
        setUserMenuOpen(false);
      }
    };
    if (transferOpen || notifOpen || userMenuOpen) {
      document.addEventListener("mousedown", handler);
    }
    return () => document.removeEventListener("mousedown", handler);
  }, [transferOpen, notifOpen, userMenuOpen]);

  useEffect(() => {
    const loadUser = () => {
      try {
        const raw = localStorage.getItem("rdesk_auth_user");
        const token = localStorage.getItem("rdesk_access_token");
        if (!raw || !token) {
          setUserInitial("U");
          setUserLabel("未登录");
          setUserAvatarUrl(null);
          setIsLoggedIn(false);
          setUserData(null);
          return;
        }
        const parsed = JSON.parse(raw) as { username?: string; role?: string; id?: string; email?: string; avatar_url?: string };
        const initial = parsed.username?.trim()?.charAt(0)?.toUpperCase() || "U";
        setUserInitial(initial);
        const username = parsed.username?.trim() || "未知用户";
        const role = parsed.role?.trim() || "user";
        setUserLabel(`${username} (${role})`);
        setUserAvatarUrl(parsed.avatar_url || null);
        setIsLoggedIn(true);
        setUserData({
          username,
          email: parsed.email || "",
          role,
          avatar_url: parsed.avatar_url,
        });
      } catch {
        setUserInitial("U");
        setUserLabel("未登录");
        setUserAvatarUrl(null);
        setIsLoggedIn(false);
        setUserData(null);
      }
    };
    loadUser();
    window.addEventListener("rdesk-auth-changed", loadUser);
    return () => window.removeEventListener("rdesk-auth-changed", loadUser);
  }, []);

  const handleLogout = async () => {
    // 解绑设备（如果已登录且有设备信息）
    if (isLoggedIn && userData) {
      try {
        await deviceService.unbindDevice(userData.id);
      } catch (err) {
        console.warn("[TitleBar] 设备解绑失败，继续登出流程:", err);
      }
    }

    localStorage.removeItem("rdesk_access_token");
    localStorage.removeItem("rdesk_auth_user");
    setIsLoggedIn(false);
    setUserInitial("U");
    setUserLabel("未登录");
    setUserAvatarUrl(null);
    setUserData(null);
    setUserMenuOpen(false);
    window.dispatchEvent(new Event("rdesk-auth-changed"));
  };

  useEffect(() => {
    void withTauriWindow(async (appWindow) => {
      const next = await appWindow.isMaximized();
      if (typeof next === "boolean") setIsMaximized(next);
    });
  }, []);

  const handleTauriDragStart = (event: React.MouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    if (event.detail > 1) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest(noDragSelector)) return;
    event.preventDefault();
    void withTauriWindow((appWindow) => appWindow.startDragging());
  };

  const handleToggleMaximize = async () => {
    await withTauriWindow(async (appWindow) => {
      await appWindow.toggleMaximize();
      const next = await appWindow.isMaximized();
      if (typeof next === "boolean") setIsMaximized(next);
    });
  };

  const handleDragDoubleClick = (event: React.MouseEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement | null;
    if (target?.closest(noDragSelector)) return;
    event.preventDefault();
    void handleToggleMaximize();
  };

  const handleMinimize = () => {
    void withTauriWindow((appWindow) => appWindow.minimize());
  };

  const handleClose = () => {
    void withTauriWindow((appWindow) => appWindow.close());
  };

  const mockTransfers = [
    { id: "1", name: "project-backup.zip", device: "办公室电脑", direction: "upload" as const, progress: 67, speed: "12.4 MB/s", size: "2.1 GB", status: "active" as const },
    { id: "2", name: "design-assets.fig", device: "设计工作站", direction: "download" as const, progress: 100, speed: "", size: "340 MB", status: "done" as const },
    { id: "3", name: "database-dump.sql", device: "服务器 A", direction: "upload" as const, progress: 34, speed: "", size: "890 MB", status: "paused" as const },
    { id: "4", name: "logs-2026-03.tar.gz", device: "服务器 B", direction: "download" as const, progress: 100, speed: "", size: "56 MB", status: "done" as const },
  ];

  const iconBtn = `flex items-center justify-center w-9 h-full transition-colors ${
    isDark
      ? "text-gray-400 hover:bg-gray-700 hover:text-gray-200"
      : "text-gray-500 hover:bg-gray-100 hover:text-gray-700"
  }`;

  return (
    <div
      className={`flex items-center h-11 border-b shrink-0 select-none ${
        isDark ? "bg-[#222] border-gray-700" : "bg-white border-gray-200/70"
      }`}
      style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
      onMouseDown={handleTauriDragStart}
      onDoubleClick={handleDragDoubleClick}
    >
      {/* Left: Sidebar toggle + Quick search */}
      <div className="flex-1 flex items-center px-2 gap-2">
        <button
          onClick={onToggleSidebar}
          className={`flex items-center justify-center w-8 h-8 rounded-md transition-colors shrink-0 ${
            isDark
              ? "text-gray-400 hover:bg-gray-700 hover:text-gray-200"
              : "text-gray-500 hover:bg-gray-100 hover:text-gray-700"
          }`}
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          title={collapsed ? "展开侧边栏" : "收起侧边栏"}
        >
          {collapsed ? (
            <PanelLeftOpen className="w-4 h-4" />
          ) : (
            <PanelLeftClose className="w-4 h-4" />
          )}
        </button>

        <div
          className={`flex items-center gap-2 px-3 py-1 rounded-md transition-all cursor-text ${
            searchFocused
              ? isDark
                ? "bg-[#2a2a2a] border border-blue-500 shadow-sm ring-2 ring-blue-500/20 w-80"
                : "bg-white border border-blue-300 shadow-sm ring-2 ring-blue-100 w-80"
              : isDark
              ? "bg-[#2a2a2a] border border-transparent hover:bg-[#333] hover:border-gray-600 w-64"
              : "bg-gray-100 border border-transparent hover:bg-gray-50 hover:border-gray-200 w-64"
          }`}
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        >
          <Search className="w-3 h-3 text-gray-400 shrink-0" />
          <input
            placeholder="搜索设备、IP 地址或 ID..."
            className={`bg-transparent outline-none placeholder-gray-500 flex-1 min-w-0 ${
              isDark ? "text-gray-200" : "text-gray-700"
            }`}
            style={{ fontSize: 12 }}
            onFocus={() => setSearchFocused(true)}
            onBlur={() => setSearchFocused(false)}
          />
          <div
            className={`flex items-center gap-0.5 px-1 py-0.5 rounded shrink-0 ${
              isDark ? "bg-gray-700 text-gray-400" : "bg-gray-200/70 text-gray-400"
            }`}
            style={{ fontSize: 10 }}
          >
            <span>Ctrl</span>
            <span>+</span>
            <span>K</span>
          </div>
        </div>

        {/* Device detail bar (collapsed mode) — after search */}
        {detailBar.collapsed && detailBar.payload && (
          <div
            className={`flex items-center gap-1 shrink-0 transition-all duration-300 ease-in-out rounded-lg px-1.5 py-0.5 ${
              isDark ? "" : "bg-gray-50 border border-gray-100"
            }`}
            style={{ WebkitAppRegion: "no-drag", opacity: 1, animation: "detailBarSlideIn 300ms ease-out" } as React.CSSProperties}
          >
            {/* Expand button */}
            <button
              onClick={detailBar.expand}
              className={`p-1 rounded-md transition-colors ${isDark ? "text-gray-400 hover:text-gray-200 hover:bg-gray-700" : "text-blue-400 hover:text-blue-600 hover:bg-blue-50"}`}
              title="展开设备信息"
            >
              <ChevronDown style={{ width: 14, height: 14 }} />
            </button>

            {/* Back */}
            <button
              onClick={detailBar.payload.onNavigateBack}
              className={`p-1 rounded-md transition-colors ${isDark ? "text-gray-400 hover:text-gray-200 hover:bg-gray-700" : "text-gray-400 hover:text-gray-700 hover:bg-gray-100"}`}
            >
              <ArrowLeft style={{ width: 14, height: 14 }} />
            </button>

            <div className={`w-px h-4 mx-0.5 ${isDark ? "bg-gray-700" : "bg-gray-200/80"}`} />

            {/* Device icon + name */}
            <div className="flex items-center gap-1.5 px-1">
              {(() => {
                const DevIcon = detailBar.payload.deviceIcon;
                return (
                  <div className={`relative w-5 h-5 rounded flex items-center justify-center ${detailBar.payload.isOnline ? (isDark ? "bg-blue-900/30" : "bg-blue-100/70") : (isDark ? "bg-gray-800" : "bg-gray-100")}`}>
                    <DevIcon style={{ width: 12, height: 12 }} className={detailBar.payload.isOnline ? (isDark ? "text-blue-600" : "text-blue-500") : "text-gray-400"} />
                    <div className={`absolute -bottom-px -right-px w-1.5 h-1.5 rounded-full border ${isDark ? "border-[#222]" : "border-gray-50"} ${detailBar.payload.isOnline ? "bg-green-500" : "bg-gray-300"}`} />
                  </div>
                );
              })()}
              <span className={`truncate max-w-[100px] ${isDark ? "text-gray-200" : "text-gray-700"}`} style={{ fontSize: 12 }}>{detailBar.payload.deviceName}</span>
              {detailBar.payload.ping !== null && (
                <span className={`${detailBar.payload.ping < 30 ? (isDark ? "text-green-600" : "text-emerald-500") : (isDark ? "text-yellow-600" : "text-amber-500")}`} style={{ fontSize: 10 }}>{detailBar.payload.ping}ms</span>
              )}
            </div>

            <div className={`w-px h-4 mx-0.5 ${isDark ? "bg-gray-700" : "bg-gray-200/80"}`} />

            {/* Tab icons */}
            <div className="flex items-center gap-0.5">
              {detailBar.payload.tabs.map((tab) => {
                const TabIcon = tab.icon;
                const isActive = detailBar.payload!.activeTab === tab.key;
                return (
                  <button
                    key={tab.key}
                    onClick={() => detailBar.payload!.setActiveTab(tab.key)}
                    className={`p-1 rounded-md transition-colors ${
                      isActive
                        ? isDark ? "bg-blue-900/30 text-blue-400" : "bg-blue-100/80 text-blue-600 shadow-sm"
                        : isDark ? "text-gray-500 hover:bg-gray-700 hover:text-gray-300" : "text-gray-400 hover:bg-white hover:text-gray-600 hover:shadow-sm"
                    }`}
                    title={tab.label}
                  >
                    <TabIcon style={{ width: 13, height: 13 }} />
                  </button>
                );
              })}
            </div>
          </div>
        )}
      </div>

      {/* Right: Status indicators + actions + window controls */}
      <div className="flex items-center h-full shrink-0" style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}>
        {/* Quick action buttons: Connections + Settings */}
        <button
          onClick={onOpenConnections}
          className={iconBtn}
          title="连接记录"
        >
          <History className="w-3.5 h-3.5" />
        </button>
        <div ref={transferRef} className="relative h-full">
          <button
            onClick={() => { setTransferOpen(!transferOpen); if (notifOpen) setNotifOpen(false); }}
            className={`relative ${iconBtn} ${transferOpen ? (isDark ? "bg-gray-700 text-gray-200" : "bg-gray-100 text-gray-700") : ""}`}
            title="传输列表"
          >
            <ArrowRightLeft className="w-3.5 h-3.5" />
            <div className="absolute top-2.5 right-1.5 w-1.5 h-1.5 rounded shrink-0 bg-blue-500" />
          </button>

          {transferOpen && (
            <div
              className={`absolute top-full right-0 mt-1 w-80 rounded-lg border z-50 overflow-hidden ${
                isDark ? "bg-[#1e1e1e] border-gray-700 shadow-[0_8px_30px_rgba(0,0,0,0.45)]" : "bg-white border-gray-200/80 shadow-[0_8px_30px_rgba(0,0,0,0.1),0_2px_8px_rgba(0,0,0,0.06)]"
              }`}
            >
              {/* Header */}
              <div className={`flex items-center justify-between px-3 py-2.5 border-b ${isDark ? "border-gray-700" : "border-gray-100"}`}>
                <div className="flex items-center gap-2">
                  <span className={isDark ? "text-gray-200" : "text-gray-800"} style={{ fontSize: 13 }}>传输列表</span>
                  <span className={`px-1.5 py-0.5 rounded-full ${isDark ? "bg-blue-900/30 text-blue-400" : "bg-blue-50 text-blue-600"}`} style={{ fontSize: 10 }}>
                    {mockTransfers.filter(t => t.status === "active" || t.status === "paused").length} 进行中
                  </span>
                </div>
                <button
                  className={`px-2 py-0.5 rounded transition-colors ${isDark ? "text-gray-400 hover:text-gray-200 hover:bg-gray-700" : "text-gray-400 hover:text-gray-600 hover:bg-gray-100"}`}
                  style={{ fontSize: 11 }}
                >
                  全部清除
                </button>
              </div>

              {/* Transfer items */}
              <div className="max-h-72 overflow-y-auto">
                {mockTransfers.map((t) => (
                  <div
                    key={t.id}
                    className={`flex items-start gap-2.5 px-3 py-2.5 transition-colors ${
                      isDark ? "hover:bg-gray-800/50" : "hover:bg-gray-50"
                    } ${t.id !== mockTransfers[mockTransfers.length - 1].id ? (isDark ? "border-b border-gray-800" : "border-b border-gray-50") : ""}`}
                  >
                    {/* Direction icon */}
                    <div className={`mt-0.5 w-6 h-6 rounded-md flex items-center justify-center shrink-0 ${
                      t.direction === "upload"
                        ? isDark ? "bg-emerald-900/30 text-emerald-400" : "bg-emerald-50 text-emerald-500"
                        : isDark ? "bg-blue-900/30 text-blue-400" : "bg-blue-50 text-blue-500"
                    }`}>
                      {t.direction === "upload"
                        ? <ArrowUpFromLine style={{ width: 12, height: 12 }} />
                        : <ArrowDownToLine style={{ width: 12, height: 12 }} />
                      }
                    </div>

                    {/* Info */}
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center justify-between gap-2">
                        <span className={`truncate ${isDark ? "text-gray-200" : "text-gray-700"}`} style={{ fontSize: 12 }}>{t.name}</span>
                        {t.status === "active" && (
                          <span className={isDark ? "text-blue-400" : "text-blue-500"} style={{ fontSize: 10 }}>{t.speed}</span>
                        )}
                        {t.status === "paused" && (
                          <Pause style={{ width: 10, height: 10 }} className={isDark ? "text-yellow-400" : "text-amber-500"} />
                        )}
                        {t.status === "done" && (
                          <CheckCircle2 style={{ width: 11, height: 11 }} className={isDark ? "text-green-400" : "text-emerald-500"} />
                        )}
                      </div>
                      <div className="flex items-center gap-1.5 mt-0.5">
                        <span className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 10 }}>{t.device}</span>
                        <span className={isDark ? "text-gray-600" : "text-gray-300"} style={{ fontSize: 10 }}>·</span>
                        <span className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 10 }}>{t.size}</span>
                      </div>
                      {/* Progress bar */}
                      {t.status !== "done" && (
                        <div className="flex items-center gap-2 mt-1.5">
                          <div className={`flex-1 h-1.5 rounded-full overflow-hidden ${isDark ? "bg-gray-700/80" : "bg-gray-100"}`} style={{ boxShadow: isDark ? "inset 0 1px 2px rgba(0,0,0,0.3)" : "inset 0 1px 2px rgba(0,0,0,0.06)" }}>
                            <div
                              className="h-full rounded-full transition-all relative overflow-hidden"
                              style={{
                                width: `${t.progress}%`,
                                background: t.status === "paused"
                                  ? isDark ? "linear-gradient(90deg, #eab308, #f59e0b)" : "linear-gradient(90deg, #f59e0b, #fbbf24)"
                                  : isDark ? "linear-gradient(90deg, #2563eb, #3b82f6)" : "linear-gradient(90deg, #3b82f6, #60a5fa)",
                                boxShadow: t.status === "paused" ? "0 0 8px rgba(245,158,11,0.35)" : "0 0 8px rgba(59,130,246,0.35)",
                              }}
                            >
                              {t.status === "active" && (
                                <div
                                  className="absolute inset-0"
                                  style={{
                                    background: "linear-gradient(90deg, transparent 0%, rgba(255,255,255,0.25) 50%, transparent 100%)",
                                    backgroundSize: "200% 100%",
                                    animation: "progressShimmer 2s ease-in-out infinite",
                                  }}
                                />
                              )}
                            </div>
                          </div>
                          <span className={`shrink-0 tabular-nums ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 10, minWidth: 28, textAlign: "right" }}>
                            {t.progress}%
                          </span>
                        </div>
                      )}
                    </div>
                  </div>
                ))}
              </div>

              {/* Footer */}
              <div className={`flex items-center justify-center px-3 py-2 border-t ${isDark ? "border-gray-700" : "border-gray-100"}`}>
                <button
                  onClick={() => { setTransferOpen(false); onOpenTransfers?.(); }}
                  className={`px-3 py-1 rounded-md transition-colors ${isDark ? "text-blue-400 hover:bg-blue-900/20" : "text-blue-500 hover:bg-blue-50"}`}
                  style={{ fontSize: 11 }}
                >
                  查看全部传输记录
                </button>
              </div>
            </div>
          )}
        </div>
        <button
          onClick={onOpenSettings}
          className={iconBtn}
          title="设置"
        >
          <Settings className="w-3.5 h-3.5" />
        </button>

        {/* Divider */}
        <div className={`w-px h-4 mx-1 ${isDark ? "bg-gray-700" : "bg-gray-200"}`} />

        {/* Notification bell */}
        <div ref={notifRef} className="relative h-full">
          <button
            onClick={() => { setNotifOpen(!notifOpen); if (transferOpen) setTransferOpen(false); }}
            className={`relative ${iconBtn} ${notifOpen ? (isDark ? "bg-gray-700 text-gray-200" : "bg-gray-100 text-gray-700") : ""}`}
            title="通知"
          >
            <Bell className="w-3.5 h-3.5" />
            {hasNotification && !notifOpen && (
              <div className="absolute top-2.5 right-2 w-1.5 h-1.5 rounded-full bg-red-500" />
            )}
          </button>

          {notifOpen && (() => {
            const notifications = [
              { id: "n1", type: "device" as const, icon: Wifi, title: "设备上线", desc: "「办公室电脑」已连接到网络", time: "2 分钟前", read: false, iconBg: isDark ? "bg-green-900/30 text-green-400" : "bg-green-50 text-green-600" },
              { id: "n2", type: "system" as const, icon: ShieldAlert, title: "安全警告", desc: "检测到「服务器 A」存在异常登录尝试 (IP: 203.0.113.42)", time: "15 分钟前", read: false, iconBg: isDark ? "bg-red-900/30 text-red-400" : "bg-red-50 text-red-500" },
              { id: "n3", type: "device" as const, icon: FolderCheck, title: "传输完成", desc: "「design-assets.fig」已成功传输到「设计工作站」", time: "32 分钟前", read: false, iconBg: isDark ? "bg-blue-900/30 text-blue-400" : "bg-blue-50 text-blue-500" },
              { id: "n4", type: "system" as const, icon: Download, title: "系统更新", desc: "R-Desk v2.4.1 版本可用，包含性能优化和安全修复", time: "1 小时前", read: true, iconBg: isDark ? "bg-purple-900/30 text-purple-400" : "bg-purple-50 text-purple-500" },
              { id: "n5", type: "device" as const, icon: WifiOff, title: "设备离线", desc: "「家庭 NAS」已断开连接", time: "2 小时前", read: true, iconBg: isDark ? "bg-orange-900/30 text-orange-400" : "bg-orange-50 text-orange-500" },
              { id: "n6", type: "system" as const, icon: UserPlus, title: "新设备请求", desc: "「小明的 MacBook」请求加入你的设备网络", time: "3 小时前", read: true, iconBg: isDark ? "bg-cyan-900/30 text-cyan-400" : "bg-cyan-50 text-cyan-600" },
              { id: "n7", type: "device" as const, icon: AlertTriangle, title: "资源告警", desc: "「服务器 B」CPU 使用率超过 90%，持续 5 分钟", time: "4 小时前", read: true, iconBg: isDark ? "bg-yellow-900/30 text-yellow-400" : "bg-yellow-50 text-yellow-600" },
              { id: "n8", type: "system" as const, icon: Info, title: "使用提示", desc: "你可以通过快捷键 Ctrl+Shift+R 快速发起远程连接", time: "昨天", read: true, iconBg: isDark ? "bg-gray-800 text-gray-400" : "bg-gray-100 text-gray-500" },
            ];

            const tabs = [
              { key: "all" as const, label: "全部" },
              { key: "unread" as const, label: "未读" },
              { key: "device" as const, label: "设备" },
              { key: "system" as const, label: "系统" },
            ];

            const filtered = notifications.filter((n) => {
              const isRead = readIds.has(n.id) || n.read;
              if (notifTab === "unread") return !isRead;
              if (notifTab === "device") return n.type === "device";
              if (notifTab === "system") return n.type === "system";
              return true;
            });

            const unreadCount = notifications.filter(n => !n.read && !readIds.has(n.id)).length;

            const markAsRead = (id: string) => setReadIds(prev => new Set(prev).add(id));
            const markAllRead = () => setReadIds(new Set(notifications.map(n => n.id)));

            return (
              <div
                className={`absolute top-full right-0 mt-1 rounded-lg border z-50 overflow-hidden ${
                  isDark ? "bg-[#1e1e1e] border-gray-700 shadow-[0_8px_30px_rgba(0,0,0,0.45)]" : "bg-white border-gray-200/80 shadow-[0_8px_30px_rgba(0,0,0,0.1),0_2px_8px_rgba(0,0,0,0.06)]"
                }`}
                style={{ width: 360 }}
              >
                {/* Header */}
                <div className={`flex items-center justify-between px-3.5 py-2.5 border-b ${isDark ? "border-gray-700" : "border-gray-100"}`}>
                  <div className="flex items-center gap-2">
                    <span className={isDark ? "text-gray-200" : "text-gray-800"} style={{ fontSize: 13 }}>消息通知</span>
                    {unreadCount > 0 && (
                      <span className={`px-1.5 py-0.5 rounded-full ${isDark ? "bg-red-900/30 text-red-400" : "bg-red-50 text-red-500"}`} style={{ fontSize: 10 }}>
                        {unreadCount} 条未读
                      </span>
                    )}
                  </div>
                  <button
                    onClick={markAllRead}
                    className={`flex items-center gap-1 px-2 py-0.5 rounded transition-colors ${isDark ? "text-gray-400 hover:text-gray-200 hover:bg-gray-700" : "text-gray-400 hover:text-gray-600 hover:bg-gray-100"}`}
                    style={{ fontSize: 11 }}
                  >
                    <Check style={{ width: 11, height: 11 }} />
                    全部已读
                  </button>
                </div>

                {/* Tab filters */}
                <div className={`flex items-center gap-0.5 px-3 py-1.5 border-b ${isDark ? "border-gray-800" : "border-gray-50"}`}>
                  {tabs.map(tab => (
                    <button
                      key={tab.key}
                      onClick={() => setNotifTab(tab.key)}
                      className={`px-2.5 py-1 rounded-md transition-colors ${
                        notifTab === tab.key
                          ? isDark ? "bg-gray-700 text-gray-200" : "bg-gray-100 text-gray-800"
                          : isDark ? "text-gray-500 hover:text-gray-300 hover:bg-gray-800" : "text-gray-400 hover:text-gray-600 hover:bg-gray-50"
                      }`}
                      style={{ fontSize: 11 }}
                    >
                      {tab.label}
                      {tab.key === "unread" && unreadCount > 0 && (
                        <span className="ml-1 text-red-500">{unreadCount}</span>
                      )}
                    </button>
                  ))}
                </div>

                {/* Notification list */}
                <div className="max-h-80 overflow-y-auto">
                  {filtered.length === 0 ? (
                    <div className="flex flex-col items-center justify-center py-10 gap-2">
                      <Bell className={`w-8 h-8 ${isDark ? "text-gray-600" : "text-gray-300"}`} />
                      <span className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 12 }}>暂无消息</span>
                    </div>
                  ) : (
                    filtered.map((n, idx) => {
                      const isUnread = !n.read && !readIds.has(n.id);
                      const NIcon = n.icon;
                      return (
                        <div
                          key={n.id}
                          onClick={() => markAsRead(n.id)}
                          className={`flex items-start gap-2.5 px-3.5 py-3 cursor-pointer transition-colors ${
                            isUnread
                              ? isDark ? "bg-blue-950/15 hover:bg-blue-950/25" : "bg-blue-50/40 hover:bg-blue-50/70"
                              : isDark ? "hover:bg-gray-800/50" : "hover:bg-gray-50"
                          } ${idx < filtered.length - 1 ? (isDark ? "border-b border-gray-800/60" : "border-b border-gray-50") : ""}`}
                        >
                          {/* Icon */}
                          <div className={`mt-0.5 w-7 h-7 rounded-lg flex items-center justify-center shrink-0 ${n.iconBg}`}>
                            <NIcon style={{ width: 14, height: 14 }} />
                          </div>

                          {/* Content */}
                          <div className="flex-1 min-w-0">
                            <div className="flex items-center justify-between gap-2">
                              <div className="flex items-center gap-1.5 min-w-0">
                                {isUnread && (
                                  <div className="w-1.5 h-1.5 rounded-full bg-blue-500 shrink-0" />
                                )}
                                <span className={`truncate ${isUnread ? (isDark ? "text-gray-100" : "text-gray-900") : (isDark ? "text-gray-300" : "text-gray-700")}`} style={{ fontSize: 12 }}>
                                  {n.title}
                                </span>
                              </div>
                              <span className={`shrink-0 ${isDark ? "text-gray-600" : "text-gray-400"}`} style={{ fontSize: 10 }}>{n.time}</span>
                            </div>
                            <p className={`mt-0.5 leading-relaxed ${isDark ? "text-gray-500" : "text-gray-500"}`} style={{ fontSize: 11 }}>
                              {n.desc}
                            </p>
                          </div>
                        </div>
                      );
                    })
                  )}
                </div>

                {/* Footer */}
                <div className={`flex items-center justify-between px-3.5 py-2 border-t ${isDark ? "border-gray-700" : "border-gray-100"}`}>
                  <button
                    className={`px-2.5 py-1 rounded-md transition-colors ${isDark ? "text-gray-500 hover:text-gray-300 hover:bg-gray-800" : "text-gray-400 hover:text-gray-600 hover:bg-gray-50"}`}
                    style={{ fontSize: 11 }}
                  >
                    消息设置
                  </button>
                  <button
                    onClick={() => setNotifOpen(false)}
                    className={`px-2.5 py-1 rounded-md transition-colors ${isDark ? "text-blue-400 hover:bg-blue-900/20" : "text-blue-500 hover:bg-blue-50"}`}
                    style={{ fontSize: 11 }}
                  >
                    查看全部消息
                  </button>
                </div>
              </div>
            );
          })()}
        </div>

        {/* User avatar with dropdown */}
        <div ref={userMenuRef} className="relative h-full">
          <button
            onClick={() => {
              if (!isLoggedIn) {
                onOpenAuth?.();
              } else {
                setUserMenuOpen(!userMenuOpen);
                if (notifOpen) setNotifOpen(false);
                if (transferOpen) setTransferOpen(false);
              }
            }}
            className={`${iconBtn} w-auto px-2 gap-2 ${userMenuOpen ? (isDark ? "bg-gray-700 text-gray-200" : "bg-gray-100 text-gray-700") : ""}`}
            title={isLoggedIn ? "用户菜单" : "登录"}
          >
            <span className={`max-w-[160px] truncate ${isDark ? "text-gray-300" : "text-gray-700"}`} style={{ fontSize: 12 }}>
              {userLabel}
            </span>
            {userAvatarUrl ? (
              <img
                src={userAvatarUrl}
                alt="Avatar"
                className="w-5 h-5 rounded-full object-cover shrink-0"
              />
            ) : (
              <div
                className="w-5 h-5 rounded-full bg-gradient-to-br from-blue-500 to-indigo-600 flex items-center justify-center text-white font-semibold shrink-0"
                style={{ fontSize: 9 }}
              >
                {userInitial}
              </div>
            )}
            {isLoggedIn && <ChevronDown className={`w-3 h-3 transition-transform ${userMenuOpen ? "rotate-180" : ""} ${isDark ? "text-gray-400" : "text-gray-400"}`} />}
          </button>

          {/* User dropdown menu */}
          {userMenuOpen && (
            <div
              className={`absolute top-full right-0 mt-1 w-56 rounded-lg border z-50 overflow-hidden ${
                isDark ? "bg-[#1e1e1e] border-gray-700 shadow-[0_8px_30px_rgba(0,0,0,0.45)]" : "bg-white border-gray-200/80 shadow-[0_8px_30px_rgba(0,0,0,0.1),0_2px_8px_rgba(0,0,0,0.06)]"
              }`}
            >
              {isLoggedIn ? (
                <>
                  {/* User info header */}
                  <div className={`px-4 py-3 border-b ${isDark ? "border-gray-700" : "border-gray-100"}`}>
                    <div className="flex items-center gap-3">
                      {userAvatarUrl ? (
                        <img src={userAvatarUrl} alt="Avatar" className="w-10 h-10 rounded-full object-cover" />
                      ) : (
                        <div className="w-10 h-10 rounded-full bg-gradient-to-br from-blue-500 to-indigo-600 flex items-center justify-center text-white font-semibold">
                          {userInitial}
                        </div>
                      )}
                      <div className="flex-1 min-w-0">
                        <div className={`truncate font-medium ${isDark ? "text-gray-100" : "text-gray-900"}`} style={{ fontSize: 13 }}>
                          {userData?.username}
                        </div>
                        <div className={`truncate ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 11 }}>
                          {userData?.email || "未设置邮箱"}
                        </div>
                      </div>
                    </div>
                  </div>

                  {/* Menu items */}
                  <div className="py-1">
                    <button
                      onClick={() => { setUserMenuOpen(false); setShowProfileModal(true); }}
                      className={`flex items-center gap-3 w-full px-4 py-2.5 transition-colors ${
                        isDark ? "text-gray-300 hover:bg-gray-800" : "text-gray-700 hover:bg-gray-50"
                      }`}
                      style={{ fontSize: 13 }}
                    >
                      <User className="w-4 h-4" />
                      个人资料
                    </button>
                    <button
                      onClick={() => { setUserMenuOpen(false); onOpenSettings?.(); }}
                      className={`flex items-center gap-3 w-full px-4 py-2.5 transition-colors ${
                        isDark ? "text-gray-300 hover:bg-gray-800" : "text-gray-700 hover:bg-gray-50"
                      }`}
                      style={{ fontSize: 13 }}
                    >
                      <Settings className="w-4 h-4" />
                      设置
                    </button>
                    <button
                      onClick={() => { setUserMenuOpen(false); onOpenAuth?.(); }}
                      className={`flex items-center gap-3 w-full px-4 py-2.5 transition-colors ${
                        isDark ? "text-gray-300 hover:bg-gray-800" : "text-gray-700 hover:bg-gray-50"
                      }`}
                      style={{ fontSize: 13 }}
                    >
                      <KeyRound className="w-4 h-4" />
                      修改密码
                    </button>
                  </div>

                  {/* Footer */}
                  <div className={`py-1 border-t ${isDark ? "border-gray-700" : "border-gray-100"}`}>
                    <button
                      onClick={handleLogout}
                      className={`flex items-center gap-3 w-full px-4 py-2.5 transition-colors ${
                        isDark ? "text-red-400 hover:bg-red-900/10" : "text-red-500 hover:bg-red-50"
                      }`}
                      style={{ fontSize: 13 }}
                    >
                      <LogOut className="w-4 h-4" />
                      退出登录
                    </button>
                  </div>
                </>
              ) : (
                <>
                  {/* Not logged in state */}
                  <div className="py-4">
                    <div className="flex flex-col items-center gap-2 mb-3">
                      <div className="w-12 h-12 rounded-full bg-gray-200 dark:bg-gray-700 flex items-center justify-center">
                        <User className="w-6 h-6 text-gray-400" />
                      </div>
                      <span className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 12 }}>未登录</span>
                    </div>
                    <button
                      onClick={() => { setUserMenuOpen(false); onOpenAuth?.(); }}
                      className="w-full mx-4 py-2 rounded-lg bg-blue-600 hover:bg-blue-500 text-white transition-colors"
                      style={{ fontSize: 13 }}
                    >
                      立即登录
                    </button>
                  </div>
                </>
              )}
            </div>
          )}
        </div>

        {/* Divider before window controls */}
        <div className={`w-px h-4 mx-1 ${isDark ? "bg-gray-700" : "bg-gray-200"}`} />

        {/* Window controls — Windows style */}
        <div className="flex items-center h-full">
          <button
            onClick={handleMinimize}
            className={`flex items-center justify-center w-11 h-full transition-colors group ${
              isDark ? "text-gray-400 hover:bg-gray-700" : "text-gray-500 hover:bg-gray-100"
            }`}
            title="最小化"
          >
            <Minus
              className={`w-4 h-4 ${isDark ? "group-hover:text-gray-200" : "group-hover:text-gray-800"}`}
            />
          </button>

          <button
            onClick={() => void handleToggleMaximize()}
            className={`flex items-center justify-center w-11 h-full transition-colors group ${
              isDark ? "text-gray-400 hover:bg-gray-700" : "text-gray-500 hover:bg-gray-100"
            }`}
            title={isMaximized ? "向下还原" : "最大化"}
          >
            {isMaximized ? (
              <div className="relative" style={{ width: 10, height: 10 }}>
                <div
                  className={`absolute border-[1.2px] rounded-[1px] ${
                    isDark
                      ? "border-gray-400 group-hover:border-gray-200"
                      : "border-gray-500 group-hover:border-gray-800"
                  }`}
                  style={{ width: 8, height: 8, top: 0, right: 0 }}
                />
                <div
                  className={`absolute border-[1.2px] rounded-[1px] ${
                    isDark
                      ? "border-gray-400 group-hover:border-gray-200 bg-[#222]"
                      : "border-gray-500 group-hover:border-gray-800 bg-white"
                  }`}
                  style={{ width: 8, height: 8, bottom: 0, left: 0 }}
                />
              </div>
            ) : (
              <Square
                className={`w-3.5 h-3.5 ${
                  isDark ? "group-hover:text-gray-200" : "group-hover:text-gray-800"
                }`}
              />
            )}
          </button>

          <button
            onClick={handleClose}
            className="flex items-center justify-center w-11 h-full text-gray-500 hover:bg-red-500 hover:text-white transition-colors rounded-tr-none"
            title="关闭"
          >
            <X className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* Profile Modal */}
      {showProfileModal && <ProfileModal isOpen={showProfileModal} onClose={() => setShowProfileModal(false)} userData={userData} isDark={isDark} />}
    </div>
  );
}

// Profile Modal Component
interface ProfileModalProps {
  isOpen: boolean;
  onClose: () => void;
  userData: { username: string; email: string; role: string; avatar_url?: string } | null;
  isDark: boolean;
}

const API_BASE = (import.meta as any).env?.VITE_RDESK_SERVER_URL ?? "http://127.0.0.1:9530/api/v1";

function ProfileModal({ isOpen, onClose, userData, isDark }: ProfileModalProps) {
  const [activeTab, setActiveTab] = useState<"profile" | "password">("profile");
  const [profileData, setProfileData] = useState({ username: "", email: "", avatar_url: "" });
  const [avatarPreview, setAvatarPreview] = useState<string | null>(null);
  const [uploadingAvatar, setUploadingAvatar] = useState(false);
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Load user profile when modal opens
  useEffect(() => {
    if (isOpen) {
      loadProfile();
    }
  }, [isOpen]);

  const loadProfile = async () => {
    try {
      const token = localStorage.getItem("rdesk_access_token");
      const resp = await fetch(`${API_BASE}/users/me`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      if (resp.ok) {
        const data = await resp.json();
        setProfileData({
          username: data.username,
          email: data.email,
          avatar_url: data.avatar_url || "",
        });
        setAvatarPreview(data.avatar_url || null);
      }
    } catch (e) {
      console.error("Failed to load profile:", e);
    }
  };

  const handleSaveProfile = async () => {
    setSaving(true);
    setMessage(null);
    try {
      const token = localStorage.getItem("rdesk_access_token");
      const resp = await fetch(`${API_BASE}/users/me`, {
        method: "PUT",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify({
          username: profileData.username,
          email: profileData.email,
        }),
      });
      if (!resp.ok) {
        const err = await resp.json();
        throw new Error(err.detail || "更新失败");
      }
      const data = await resp.json();
      setProfileData({ username: data.username, email: data.email, avatar_url: data.avatar_url || "" });
      setMessage({ type: "success", text: "个人资料已更新" });
      // Update localStorage
      const storedUser = JSON.parse(localStorage.getItem("rdesk_auth_user") || "{}");
      localStorage.setItem("rdesk_auth_user", JSON.stringify({
        ...storedUser,
        username: data.username,
        email: data.email,
      }));
      window.dispatchEvent(new Event("rdesk-auth-changed"));
    } catch (e) {
      setMessage({ type: "error", text: e instanceof Error ? e.message : "更新失败" });
    } finally {
      setSaving(false);
    }
  };

  const handleAvatarClick = () => {
    fileInputRef.current?.click();
  };

  const handleAvatarChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;

    // Validate file type
    if (!file.type.startsWith("image/")) {
      setMessage({ type: "error", text: "请选择图片文件" });
      return;
    }

    // Validate file size (5MB)
    if (file.size > 5 * 1024 * 1024) {
      setMessage({ type: "error", text: "图片大小不能超过 5MB" });
      return;
    }

    // Show preview
    const reader = new FileReader();
    reader.onloadend = () => {
      setAvatarPreview(reader.result as string);
    };
    reader.readAsDataURL(file);

    // Upload
    setUploadingAvatar(true);
    setMessage(null);
    try {
      const token = localStorage.getItem("rdesk_access_token");
      const formData = new FormData();
      formData.append("file", file);

      const resp = await fetch(`${API_BASE}/users/me/avatar`, {
        method: "POST",
        headers: { Authorization: `Bearer ${token}` },
        body: formData,
      });

      if (!resp.ok) {
        throw new Error("上传失败");
      }

      const data = await resp.json();
      setProfileData(prev => ({ ...prev, avatar_url: data.avatar_url }));
      setMessage({ type: "success", text: "头像已更新" });
    } catch (e) {
      setMessage({ type: "error", text: "头像上传失败" });
    } finally {
      setUploadingAvatar(false);
    }
  };

  const handleDeleteAvatar = async () => {
    setUploadingAvatar(true);
    try {
      const token = localStorage.getItem("rdesk_access_token");
      await fetch(`${API_BASE}/users/me/avatar`, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${token}` },
      });
      setAvatarPreview(null);
      setProfileData(prev => ({ ...prev, avatar_url: "" }));
      setMessage({ type: "success", text: "头像已删除" });
    } catch (e) {
      setMessage({ type: "error", text: "删除失败" });
    } finally {
      setUploadingAvatar(false);
    }
  };

  const handleChangePassword = async () => {
    if (newPassword !== confirmPassword) {
      setMessage({ type: "error", text: "两次输入的密码不一致" });
      return;
    }
    if (newPassword.length < 8) {
      setMessage({ type: "error", text: "新密码至少 8 位" });
      return;
    }
    setSaving(true);
    setMessage(null);
    try {
      const token = localStorage.getItem("rdesk_access_token");
      const resp = await fetch(`${API_BASE}/users/me/change-password`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify({
          current_password: currentPassword,
          new_password: newPassword,
        }),
      });
      if (!resp.ok) {
        const err = await resp.json();
        throw new Error(err.detail || "修改失败");
      }
      setMessage({ type: "success", text: "密码已修改" });
      setCurrentPassword("");
      setNewPassword("");
      setConfirmPassword("");
    } catch (e) {
      setMessage({ type: "error", text: e instanceof Error ? e.message : "修改失败" });
    } finally {
      setSaving(false);
    }
  };

  if (!isOpen) return null;

  return (
    <div
      className={`fixed inset-0 z-50 flex items-center justify-center transition-opacity duration-200 ${
        isDark ? "bg-black/60" : "bg-black/30"
      }`}
      onClick={onClose}
    >
      <div
        className={`relative rounded-2xl border transition-all duration-200 overflow-hidden ${
          isDark ? "bg-[#1e1e1e] border-gray-700 shadow-[0_12px_40px_rgba(0,0,0,0.5)]" : "bg-white border-gray-200/80 shadow-[0_12px_40px_rgba(0,0,0,0.12),0_4px_12px_rgba(0,0,0,0.06)]"
        }`}
        style={{ width: 480 }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header with tabs on the right */}
        <div className={`flex items-center justify-between px-6 py-3 border-b ${isDark ? "border-gray-700" : "border-gray-100"}`}>
          <h2 className={`font-semibold ${isDark ? "text-gray-100" : "text-gray-900"}`} style={{ fontSize: 16 }}>
            个人资料
          </h2>
          <div className="flex items-center gap-2">
            {/* Tabs */}
            <div className={`flex items-center gap-1 ${isDark ? "bg-gray-800" : "bg-gray-100"} rounded-lg p-0.5`}>
              <button
                onClick={() => { setActiveTab("profile"); setMessage(null); }}
                className={`px-3 py-1.5 rounded-md transition-colors ${
                  activeTab === "profile"
                    ? isDark ? "bg-[#1e1e1e] text-blue-400" : "bg-white text-blue-600 shadow-sm"
                    : isDark ? "text-gray-400 hover:text-gray-200" : "text-gray-500 hover:text-gray-700"
                }`}
                style={{ fontSize: 12 }}
              >
                个人信息
              </button>
              <button
                onClick={() => { setActiveTab("password"); setMessage(null); }}
                className={`px-3 py-1.5 rounded-md transition-colors ${
                  activeTab === "password"
                    ? isDark ? "bg-[#1e1e1e] text-blue-400" : "bg-white text-blue-600 shadow-sm"
                    : isDark ? "text-gray-400 hover:text-gray-200" : "text-gray-500 hover:text-gray-700"
                }`}
                style={{ fontSize: 12 }}
              >
                修改密码
              </button>
            </div>
            {/* Close button */}
            <button
              onClick={onClose}
              className={`p-1.5 rounded-lg transition-colors ${isDark ? "text-gray-400 hover:text-gray-200 hover:bg-gray-700" : "text-gray-400 hover:text-gray-600 hover:bg-gray-100"}`}
            >
              <X className="w-4 h-4" />
            </button>
          </div>
        </div>

        {/* Content */}
        <div className="px-6 py-5">
          {activeTab === "profile" ? (
            <div className="space-y-4">
              {/* Avatar with upload */}
              <div className="flex items-center gap-4 pb-4 border-b" style={{ fontSize: 13 }}>
                <div className="relative group">
                  <input
                    ref={fileInputRef}
                    type="file"
                    accept="image/*"
                    onChange={handleAvatarChange}
                    className="hidden"
                  />
                  <div
                    onClick={handleAvatarClick}
                    className={`w-20 h-20 rounded-full overflow-hidden cursor-pointer relative ${
                      isDark ? "ring-2 ring-gray-700 hover:ring-blue-500" : "ring-2 ring-gray-200 hover:ring-blue-400"
                    } transition-all`}
                  >
                    {avatarPreview ? (
                      <img src={avatarPreview} alt="Avatar" className="w-full h-full object-cover" />
                    ) : (
                      <div className="w-full h-full bg-gradient-to-br from-blue-500 to-indigo-600 flex items-center justify-center text-white font-bold" style={{ fontSize: 28 }}>
                        {profileData.username?.charAt(0).toUpperCase() || "U"}
                      </div>
                    )}
                    <div className="absolute inset-0 bg-black/50 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
                      <Edit className="w-5 h-5 text-white" />
                    </div>
                  </div>
                  {uploadingAvatar && (
                    <div className="absolute inset-0 flex items-center justify-center bg-black/50 rounded-full">
                      <div className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
                    </div>
                  )}
                </div>
                <div className="flex-1">
                  <div className={`font-medium ${isDark ? "text-gray-100" : "text-gray-900"}`}>{profileData.username}</div>
                  <div className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 12 }}>
                    {userData?.role === "admin" ? "管理员" : "普通用户"}
                  </div>
                  <div className="flex items-center gap-2 mt-2">
                    <button
                      onClick={handleAvatarClick}
                      disabled={uploadingAvatar}
                      className={`text-xs px-2 py-1 rounded transition-colors ${
                        isDark
                          ? "bg-gray-700 text-gray-300 hover:bg-gray-600"
                          : "bg-gray-100 text-gray-600 hover:bg-gray-200"
                      } ${uploadingAvatar ? "opacity-50 cursor-not-allowed" : ""}`}
                    >
                      更换头像
                    </button>
                    {avatarPreview && (
                      <button
                        onClick={handleDeleteAvatar}
                        disabled={uploadingAvatar}
                        className={`text-xs px-2 py-1 rounded transition-colors ${
                          isDark
                            ? "bg-red-900/30 text-red-400 hover:bg-red-900/50"
                            : "bg-red-50 text-red-500 hover:bg-red-100"
                        } ${uploadingAvatar ? "opacity-50 cursor-not-allowed" : ""}`}
                      >
                        删除
                      </button>
                    )}
                  </div>
                </div>
              </div>

              {/* Username */}
              <div>
                <label className={`block mb-1.5 ${isDark ? "text-gray-400" : "text-gray-500"}`} style={{ fontSize: 12 }}>
                  用户名
                </label>
                <input
                  type="text"
                  value={profileData.username}
                  onChange={(e) => setProfileData(prev => ({ ...prev, username: e.target.value }))}
                  className={`w-full px-3 py-2.5 rounded-lg border outline-none transition-all ${
                    isDark
                      ? "bg-[#2a2a2a] border-gray-600 text-gray-200 placeholder-gray-500 focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20"
                      : "bg-gray-50 border-gray-200 text-gray-900 placeholder-gray-400 focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
                  }`}
                  style={{ fontSize: 13 }}
                />
              </div>

              {/* Email */}
              <div>
                <label className={`block mb-1.5 ${isDark ? "text-gray-400" : "text-gray-500"}`} style={{ fontSize: 12 }}>
                  邮箱地址
                </label>
                <input
                  type="email"
                  value={profileData.email}
                  onChange={(e) => setProfileData(prev => ({ ...prev, email: e.target.value }))}
                  className={`w-full px-3 py-2.5 rounded-lg border outline-none transition-all ${
                    isDark
                      ? "bg-[#2a2a2a] border-gray-600 text-gray-200 placeholder-gray-500 focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20"
                      : "bg-gray-50 border-gray-200 text-gray-900 placeholder-gray-400 focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
                  }`}
                  style={{ fontSize: 13 }}
                />
              </div>

              {/* Role (read-only) */}
              <div>
                <label className={`block mb-1.5 ${isDark ? "text-gray-400" : "text-gray-500"}`} style={{ fontSize: 12 }}>
                  账户类型
                </label>
                <div className={`px-3 py-2.5 rounded-lg border ${isDark ? "bg-gray-800 border-gray-700 text-gray-400" : "bg-gray-50 border-gray-200 text-gray-400"}`} style={{ fontSize: 13 }}>
                  {userData?.role === "admin" ? "管理员" : "普通用户"}
                </div>
              </div>
            </div>
          ) : (
            <div className="space-y-4">
              {/* Current Password */}
              <div>
                <label className={`block mb-1.5 ${isDark ? "text-gray-400" : "text-gray-500"}`} style={{ fontSize: 12 }}>
                  当前密码
                </label>
                <input
                  type="password"
                  value={currentPassword}
                  onChange={(e) => setCurrentPassword(e.target.value)}
                  placeholder="请输入当前密码"
                  className={`w-full px-3 py-2.5 rounded-lg border outline-none transition-all ${
                    isDark
                      ? "bg-[#2a2a2a] border-gray-600 text-gray-200 placeholder-gray-500 focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20"
                      : "bg-gray-50 border-gray-200 text-gray-900 placeholder-gray-400 focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
                  }`}
                  style={{ fontSize: 13 }}
                />
              </div>

              {/* New Password */}
              <div>
                <label className={`block mb-1.5 ${isDark ? "text-gray-400" : "text-gray-500"}`} style={{ fontSize: 12 }}>
                  新密码
                </label>
                <input
                  type="password"
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  placeholder="至少 8 位"
                  className={`w-full px-3 py-2.5 rounded-lg border outline-none transition-all ${
                    isDark
                      ? "bg-[#2a2a2a] border-gray-600 text-gray-200 placeholder-gray-500 focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20"
                      : "bg-gray-50 border-gray-200 text-gray-900 placeholder-gray-400 focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
                  }`}
                  style={{ fontSize: 13 }}
                />
              </div>

              {/* Confirm Password */}
              <div>
                <label className={`block mb-1.5 ${isDark ? "text-gray-400" : "text-gray-500"}`} style={{ fontSize: 12 }}>
                  确认新密码
                </label>
                <input
                  type="password"
                  value={confirmPassword}
                  onChange={(e) => setConfirmPassword(e.target.value)}
                  placeholder="再次输入新密码"
                  className={`w-full px-3 py-2.5 rounded-lg border outline-none transition-all ${
                    isDark
                      ? "bg-[#2a2a2a] border-gray-600 text-gray-200 placeholder-gray-500 focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20"
                      : "bg-gray-50 border-gray-200 text-gray-900 placeholder-gray-400 focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
                  }`}
                  style={{ fontSize: 13 }}
                />
              </div>

              {/* Password strength indicator */}
              {newPassword && (
                <div className="flex items-center gap-2">
                  <span className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 12 }}>密码强度：</span>
                  <div className="flex gap-1">
                    {[1, 2, 3, 4].map((level) => {
                      const strength = newPassword.length >= 12 ? 4 : newPassword.length >= 8 ? 3 : 2;
                      const colors = ["bg-red-500", "bg-yellow-500", "bg-blue-500", "bg-green-500"];
                      return (
                        <div
                          key={level}
                          className={`h-1 w-8 rounded-full transition-colors ${
                            level <= strength ? colors[strength - 1] : isDark ? "bg-gray-700" : "bg-gray-200"
                          }`}
                        />
                      );
                    })}
                  </div>
                  <span className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 11 }}>
                    {newPassword.length >= 12 ? "强" : newPassword.length >= 8 ? "中" : "弱"}
                  </span>
                </div>
              )}
            </div>
          )}

          {/* Message */}
          {message && (
            <div className={`mt-4 p-3 rounded-lg text-center ${
              message.type === "success"
                ? isDark ? "bg-green-900/20 text-green-400 border border-green-800" : "bg-green-50 text-green-600 border border-green-200"
                : isDark ? "bg-red-900/20 text-red-400 border border-red-800" : "bg-red-50 text-red-500 border border-red-200"
            }`} style={{ fontSize: 12 }}>
              {message.text}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className={`flex items-center justify-end gap-3 px-6 py-4 border-t ${isDark ? "border-gray-700" : "border-gray-100"}`}>
          <button
            onClick={onClose}
            className={`px-4 py-2 rounded-lg transition-colors ${isDark ? "text-gray-400 hover:bg-gray-800" : "text-gray-500 hover:bg-gray-100"}`}
            style={{ fontSize: 13 }}
          >
            取消
          </button>
          <button
            onClick={activeTab === "profile" ? handleSaveProfile : handleChangePassword}
            disabled={saving}
            className="px-5 py-2 rounded-lg bg-blue-600 hover:bg-blue-500 text-white transition-colors disabled:opacity-50"
            style={{ fontSize: 13 }}
          >
            {saving ? "保存中..." : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}
