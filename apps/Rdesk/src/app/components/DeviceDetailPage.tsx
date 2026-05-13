import { useCallback, useState, useEffect, useRef } from "react";
import { useParams, useNavigate } from "react-router";
import { type Device, useDeviceById, useDevices } from "./deviceData";
import {
  launchRemoteApplicationForDevice,
  launchRemoteDisplayForDevice,
  prepareRemoteApplicationCatalogForDevice,
  type RemoteApplicationCatalogResult,
} from "../services/remoteDisplayLauncher";
import {
  getProbeSnapshot,
  getSessionSnapshot,
  stopSession,
  type CaptureSource,
  type CaptureSourceSelection,
  type ProbeSnapshot,
  type SessionRuntimeSnapshot,
} from "../services/ipcSessionService";
import { isTauriRuntime } from "../utils/runtime";
import {
  ArrowLeft,
  Monitor,
  FolderOpen,
  AppWindow,
  Wifi,
  WifiOff,
  MapPin,
  Clock,
  Cpu,
  HardDrive,
  MemoryStick,
  Power,
  RefreshCw,
  Lock,
  Copy,
  Star,
  MoreVertical,
  Keyboard,
  Mouse,
  Volume2,
  VolumeX,
  Clipboard,
  Maximize2,
  Minimize2,
  Send,
  Pause,
  Play,
  Upload,
  Download,
  File,
  Folder,
  ChevronRight,
  Globe,
  FileText,
  Image,
  Music,
  Terminal,
  Presentation,
  Database,
  Code,
  Settings,
  ExternalLink,
  Activity,
  ArrowUp,
  Home,
  Search,
  LayoutGrid,
  List,
  Plus,
  Laptop,
  Server,
  Smartphone,
  Trash2,
  Scissors,
  ClipboardPaste,
  Edit3,
  Info,
  ArrowRightLeft,
  ChevronUp,
  Loader2,
  AlertCircle,
} from "lucide-react";
import { useTheme } from "./ThemeContext";
import { useDetailBar } from "./DetailBarContext";

type TabType = "remote" | "files" | "apps";

const remoteFiles = [
  { name: "Documents", type: "folder" as const, size: "—", modified: "2026-03-03" },
  { name: "Downloads", type: "folder" as const, size: "—", modified: "2026-03-04" },
  { name: "Desktop", type: "folder" as const, size: "—", modified: "2026-03-04" },
  { name: "report_2026.pdf", type: "file" as const, size: "2.3 MB", modified: "2026-03-01" },
  { name: "config.json", type: "file" as const, size: "12 KB", modified: "2026-02-28" },
  { name: "backup.tar.gz", type: "file" as const, size: "890 MB", modified: "2026-02-27" },
  { name: "screenshot.png", type: "file" as const, size: "4.1 MB", modified: "2026-03-04" },
];

const allRemoteFiles = [
  { name: "Documents", type: "folder" as const, size: "—", modified: "2026-03-03", fileKind: "文件夹" },
  { name: "Downloads", type: "folder" as const, size: "—", modified: "2026-03-04", fileKind: "文件夹" },
  { name: "Desktop", type: "folder" as const, size: "—", modified: "2026-03-04", fileKind: "文件夹" },
  { name: "Pictures", type: "folder" as const, size: "—", modified: "2026-03-02", fileKind: "文件夹" },
  { name: "Music", type: "folder" as const, size: "—", modified: "2026-02-15", fileKind: "文件夹" },
  { name: "Videos", type: "folder" as const, size: "—", modified: "2026-02-20", fileKind: "文件夹" },
  { name: "report_2026.pdf", type: "file" as const, size: "2.3 MB", modified: "2026-03-01", fileKind: "PDF 文档" },
  { name: "config.json", type: "file" as const, size: "12 KB", modified: "2026-02-28", fileKind: "JSON 文件" },
  { name: "backup.tar.gz", type: "file" as const, size: "890 MB", modified: "2026-02-27", fileKind: "压缩包" },
  { name: "screenshot.png", type: "file" as const, size: "4.1 MB", modified: "2026-03-04", fileKind: "PNG 图片" },
  { name: "notes.txt", type: "file" as const, size: "4 KB", modified: "2026-03-04", fileKind: "文本文件" },
  { name: "presentation.pptx", type: "file" as const, size: "18 MB", modified: "2026-03-03", fileKind: "演示文稿" },
  { name: "database.sql", type: "file" as const, size: "156 KB", modified: "2026-02-25", fileKind: "SQL 文件" },
  { name: "logo.jpg", type: "file" as const, size: "320 KB", modified: "2026-03-02", fileKind: "JPEG 图片" },
];

const localFiles = [
  { name: "Projects", type: "folder" as const, size: "—", modified: "2026-03-04" },
  { name: "Pictures", type: "folder" as const, size: "—", modified: "2026-03-03" },
  { name: "Music", type: "folder" as const, size: "—", modified: "2026-02-20" },
  { name: "presentation.pptx", type: "file" as const, size: "18 MB", modified: "2026-03-04" },
  { name: "notes.txt", type: "file" as const, size: "4 KB", modified: "2026-03-04" },
  { name: "dataset.csv", type: "file" as const, size: "56 MB", modified: "2026-03-02" },
];

export function DeviceDetailPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const { devices, loading } = useDevices();
  const device = useDeviceById(id, devices);
  const [activeTab, setActiveTab] = useState<TabType>("remote");
  const { isDark } = useTheme();
  const detailBar = useDetailBar();

  const Icon = device?.icon || Monitor;
  const isOnline = device?.status === "online";
  const tabs: { key: TabType; label: string; icon: typeof Monitor }[] = [
    { key: "remote", label: "远程桌面", icon: Monitor },
    { key: "files", label: "文件传输", icon: FolderOpen },
    { key: "apps", label: "远程应用", icon: AppWindow },
  ];

  const handleCollapse = () => {
    if (!device) return;
    detailBar.collapse({
      deviceName: device.name,
      deviceIcon: Icon,
      isOnline,
      ping: device.ping,
      tabs,
      activeTab,
      setActiveTab: (key: string) => setActiveTab(key as TabType),
      onNavigateBack: () => navigate("/devices"),
    });
  };

  // Keep context payload in sync with local activeTab
  useEffect(() => {
    if (detailBar.collapsed && detailBar.payload && device) {
      detailBar.collapse({
        ...detailBar.payload,
        activeTab,
        setActiveTab: (key: string) => setActiveTab(key as TabType),
      });
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTab]);

  // Clean up context when leaving this page
  useEffect(() => {
    return () => {
      detailBar.reset();
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Early returns after all hooks
  if (loading) {
    return <div className="flex items-center justify-center h-full">加载设备中...</div>;
  }

  if (!device) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="text-center">
          <div className="text-gray-400 mb-2" style={{ fontSize: 48 }}>?</div>
          <div className="text-gray-600" style={{ fontSize: 16 }}>设备未找到</div>
          <button
            onClick={() => navigate("/devices")}
            className="mt-3 px-4 py-2 rounded-lg bg-blue-600 text-white hover:bg-blue-500 transition-colors"
            style={{ fontSize: 13 }}
          >
            返回设备列表
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      {/* Top bar: device info + tabs — slides up when collapsed into TitleBar */}
      <div
        className={`shrink-0 border-b transition-all duration-300 ease-in-out overflow-hidden ${
          isDark ? "bg-[#1e1e1e] border-gray-700" : "bg-white border-gray-200/70"
        }`}
        style={{ height: detailBar.collapsed ? 0 : 60, opacity: detailBar.collapsed ? 0 : 1 }}
      >
        <div className="flex items-center gap-4 px-6" style={{ height: 60 }}>
          <button
            onClick={() => navigate("/devices")}
            className={`p-1.5 rounded-md transition-colors ${isDark ? "text-gray-400 hover:text-gray-200 hover:bg-gray-800" : "text-gray-400 hover:text-gray-700 hover:bg-gray-100"}`}
          >
            <ArrowLeft style={{ width: 16, height: 16 }} />
          </button>

          <div className={`relative w-9 h-9 rounded-lg flex items-center justify-center ${isOnline ? (isDark ? "bg-blue-900/30" : "bg-blue-50") : (isDark ? "bg-gray-800" : "bg-gray-100")}`}>
            <Icon style={{ width: 18, height: 18 }} className={isOnline ? "text-blue-600" : "text-gray-400"} />
            <div className={`absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full border-2 ${isDark ? "border-[#1e1e1e]" : "border-white"} ${isOnline ? "bg-green-500" : "bg-gray-300"}`} />
          </div>

          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className={`font-medium truncate ${isDark ? "text-gray-100" : "text-gray-900"}`} style={{ fontSize: 15 }}>{device.name}</span>
              {device.favorite && <Star className="w-3.5 h-3.5 text-yellow-500 fill-yellow-500 shrink-0" />}
              <span className={`px-1.5 py-0.5 rounded text-white shrink-0 ${isOnline ? "bg-green-500" : "bg-gray-400"}`} style={{ fontSize: 10 }}>
                {isOnline ? "在线" : "离线"}
              </span>
            </div>
            <div className="flex items-center gap-3 mt-0.5">
              <span className={`font-mono ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 11 }}>{device.deviceId}</span>
              <span className={isDark ? "text-gray-600" : "text-gray-300"} style={{ fontSize: 11 }}>·</span>
              <span className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 11 }}>{device.os}</span>
              <span className={isDark ? "text-gray-600" : "text-gray-300"} style={{ fontSize: 11 }}>·</span>
              <span className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 11 }}>{device.ip}</span>
              {device.ping !== null && (
                <>
                  <span className={isDark ? "text-gray-600" : "text-gray-300"} style={{ fontSize: 11 }}>·</span>
                  <span className={`${device.ping < 30 ? "text-green-600" : "text-yellow-600"}`} style={{ fontSize: 11 }}>
                    {device.ping}ms
                  </span>
                </>
              )}
            </div>
          </div>

          {/* Tab buttons */}
          <div className="flex items-center gap-1 shrink-0">
            {tabs.map((tab) => {
              const TabIcon = tab.icon;
              const isActive = activeTab === tab.key;
              return (
                <button
                  key={tab.key}
                  onClick={() => setActiveTab(tab.key)}
                  className={`flex items-center gap-1.5 px-3 py-1.5 rounded-md transition-colors ${
                    isActive
                      ? isDark
                        ? "bg-blue-900/30 text-blue-400"
                        : "bg-blue-50 text-blue-600"
                      : isDark
                        ? "text-gray-400 hover:bg-gray-800 hover:text-gray-200"
                        : "text-gray-500 hover:bg-gray-100 hover:text-gray-700"
                  }`}
                  style={{ fontSize: 12 }}
                >
                  <TabIcon style={{ width: 14, height: 14 }} />
                  {tab.label}
                </button>
              );
            })}
          </div>

          <button className={`p-1.5 rounded-md transition-colors ${isDark ? "text-gray-400 hover:text-gray-200 hover:bg-gray-800" : "text-gray-400 hover:text-gray-700 hover:bg-gray-100"}`}>
            <MoreVertical style={{ width: 16, height: 16 }} />
          </button>

          {/* Collapse button */}
          <button
            onClick={handleCollapse}
            className={`p-1 rounded-md transition-colors ${isDark ? "text-gray-500 hover:text-gray-300 hover:bg-gray-800" : "text-gray-400 hover:text-gray-600 hover:bg-gray-100"}`}
            title="收起到标题栏"
          >
            <ChevronUp style={{ width: 14, height: 14 }} />
          </button>
        </div>
      </div>

      {/* Tab content */}
      <div className="flex-1 overflow-hidden">
        {activeTab === "remote" && <RemoteTab device={device} />}
        {activeTab === "files" && <FilesTab device={device} devices={devices} />}
        {activeTab === "apps" && <AppsTab device={device} />}
      </div>

      {/* Performance monitoring footer */}
      {isOnline && device.cpu !== null && (
        <PerformanceFooter device={device} />
      )}
    </div>
  );
}

/* ======================== Remote Desktop Tab ======================== */
function RemoteTab({ device }: { device: Device }) {
  const { isDark } = useTheme();
  const navigate = useNavigate();
  const [muted, setMuted] = useState(false);
  const latency = device.ping ?? 0;
  const [elapsed, setElapsed] = useState(0);
  const [connected, setConnected] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const activeSessionIdRef = useRef<string | null>(null);
  const [remoteWindowLabel, setRemoteWindowLabel] = useState<string | null>(null);
  const [sessionSnapshot, setSessionSnapshot] = useState<SessionRuntimeSnapshot | null>(null);
  const [probeSnapshot, setProbeSnapshot] = useState<ProbeSnapshot | null>(null);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const isOnline = device.status === "online";
  const isLanP2PRemote = device.p2pAvailable && !device.isLocal;
  const preferredTransport = device.p2pAvailable
    ? "quic"
    : device.os.toLowerCase().includes("quic")
      ? "quic"
      : "webrtc";

  useEffect(() => {
    if (!connected) return;
    const timer = setInterval(() => {
      setElapsed((e) => e + 1);
    }, 1000);
    return () => clearInterval(timer);
  }, [connected]);

  useEffect(() => {
    if (!activeSessionId) return;
    let cancelled = false;

    const refresh = async () => {
      const [sessionResult, probeResult] = await Promise.allSettled([
        getSessionSnapshot(activeSessionId),
        getProbeSnapshot(activeSessionId),
      ]);

      if (cancelled) return;

      if (sessionResult.status === "fulfilled") {
        setSessionSnapshot(sessionResult.value);
        if (sessionResult.value.last_error) setConnectionError(sessionResult.value.last_error);
      } else {
        setConnectionError(sessionResult.reason instanceof Error ? sessionResult.reason.message : "Failed to read session state");
      }

      if (probeResult.status === "fulfilled") {
        setProbeSnapshot(probeResult.value);
        if (probeResult.value.last_error) setConnectionError(probeResult.value.last_error);
      }
    };

    void refresh();
    const timer = window.setInterval(() => void refresh(), 1000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [activeSessionId]);

  const formatTime = (s: number) => {
    const m = Math.floor(s / 60);
    const sec = s % 60;
    return `${m.toString().padStart(2, "0")}:${sec.toString().padStart(2, "0")}`;
  };

  const handleStartRemote = async () => {
    setLaunching(true);
    setConnectionError(null);
    try {
      const result = await launchRemoteDisplayForDevice(device.deviceId, {
        transportKind: preferredTransport,
        targetDeviceName: device.name,
        targetOs: device.os,
        targetIp: device.ip,
        lanP2P: isLanP2PRemote,
      });
      activeSessionIdRef.current = result.sessionId;
      setActiveSessionId(result.sessionId);
      setRemoteWindowLabel(result.windowLabel);
      setSessionSnapshot(null);
      setProbeSnapshot(null);
      setElapsed(0);
      setConnected(true);
      if (result.mode === "route") navigate(`/session/${result.sessionId}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Open remote display failed";
      setConnectionError(message);
      alert(message);
    } finally {
      setLaunching(false);
    }
  };

  const handleDisconnect = async () => {
    const sessionId = activeSessionIdRef.current ?? activeSessionId;
    activeSessionIdRef.current = null;
    setConnected(false);
    setActiveSessionId(null);
    setRemoteWindowLabel(null);
    setSessionSnapshot(null);
    setProbeSnapshot(null);
    setElapsed(0);
    if (!sessionId) return;
    try {
      await stopSession(sessionId);
    } catch (error) {
      setConnectionError(error instanceof Error ? error.message : "Stop session failed");
    }
  };

  const fpsLabel = probeSnapshot?.current_fps == null ? "probing" : `${probeSnapshot.current_fps.toFixed(1)} fps`;
  const bitrateLabel = probeSnapshot?.bitrate_mbps == null ? "-" : `${probeSnapshot.bitrate_mbps.toFixed(2)} Mbps`;
  const frameSizeLabel =
    probeSnapshot?.latest_frame_width && probeSnapshot?.latest_frame_height
      ? `${probeSnapshot.latest_frame_width}x${probeSnapshot.latest_frame_height}`
      : probeSnapshot?.media_probe_width && probeSnapshot?.media_probe_height
        ? `${probeSnapshot.media_probe_width}x${probeSnapshot.media_probe_height}`
        : "-";
  const sessionStateLabel = sessionSnapshot?.state ?? (connected ? "connecting" : "idle");
  const decodedFrames = probeSnapshot?.frames_decoded ?? 0;

  if (!isOnline) {
    return (
      <div className={`flex items-center justify-center h-full p-6 ${isDark ? "bg-[#1a1a1a]" : "bg-[#f0f2f5]"}`}>
        <div className={`w-full max-w-[520px] rounded-xl border p-6 text-center shadow-sm ${isDark ? "bg-[#202020] border-gray-700" : "bg-white border-gray-200"}`}>
          <WifiOff className={`w-12 h-12 mx-auto mb-3 ${isDark ? "text-gray-600" : "text-gray-300"}`} />
          <div className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 16 }}>设备当前离线</div>
          <div className={`mt-1 ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 13 }}>最后在线: {device.lastSeen}</div>
        </div>
      </div>
    );
  }

  if (!connected) {
    return (
      <div className={`flex items-center justify-center h-full p-6 ${isDark ? "bg-[#1a1a1a]" : "bg-[#f0f2f5]"}`}>
        <div className={`w-full max-w-[520px] rounded-xl border p-6 text-center shadow-sm ${isDark ? "bg-[#202020] border-gray-700" : "bg-white border-gray-200"}`}>
          <div className={`w-16 h-16 rounded-2xl flex items-center justify-center mx-auto mb-4 ${isDark ? "bg-blue-900/30" : "bg-blue-50"}`}>
            <Monitor className="w-8 h-8 text-blue-600" />
          </div>
          <div className={`mb-1 ${isDark ? "text-gray-200" : "text-gray-800"}`} style={{ fontSize: 18 }}>连接到 {device.name}</div>
          <div className={`mb-6 ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 13 }}>{device.os} · {device.ip} · 延迟 {device.ping}ms</div>
          <div className={`grid grid-cols-2 gap-3 mb-5 text-left ${isDark ? "text-gray-300" : "text-gray-700"}`} style={{ fontSize: 12 }}>
            <div className={`rounded-lg px-3 py-2 ${isDark ? "bg-[#2a2a2a]" : "bg-gray-50"}`}>
              <div className={isDark ? "text-gray-500" : "text-gray-400"}>发现来源</div>
              <div className="mt-1 font-medium">{device.sourceLabel}</div>
            </div>
            <div className={`rounded-lg px-3 py-2 ${isDark ? "bg-[#2a2a2a]" : "bg-gray-50"}`}>
              <div className={isDark ? "text-gray-500" : "text-gray-400"}>连接方式</div>
              <div className="mt-1 font-medium">{isLanP2PRemote ? "P2P LAN 自动接受" : "mrd-service 会话"}</div>
            </div>
          </div>
          <button
            onClick={() => void handleStartRemote()}
            disabled={launching}
            className="w-full px-8 py-2.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-white transition-colors shadow-sm disabled:cursor-not-allowed disabled:opacity-60"
            style={{ fontSize: 14 }}
          >
            发起远程连接
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full bg-[#1a1a2e]">
      {/* Toolbar */}
      <div className="flex items-center gap-1 px-3 py-1.5 bg-[#232340] border-b border-white/10 shrink-0">
        <ToolbarBtn icon={<Mouse className="w-3.5 h-3.5" />} label="鼠标" />
        <ToolbarBtn icon={<Keyboard className="w-3.5 h-3.5" />} label="键盘" />
        <ToolbarBtn
          icon={muted ? <VolumeX className="w-3.5 h-3.5" /> : <Volume2 className="w-3.5 h-3.5" />}
          label={muted ? "静音" : "音频"}
          onClick={() => setMuted(!muted)}
          active={!muted}
        />
        <ToolbarBtn icon={<Clipboard className="w-3.5 h-3.5" />} label="剪贴板" />
        <div className="w-px h-4 bg-white/10 mx-1" />
        <ToolbarBtn icon={<Lock className="w-3.5 h-3.5" />} label="锁屏" />
        <ToolbarBtn icon={<RefreshCw className="w-3.5 h-3.5" />} label="刷新" />
        <ToolbarBtn icon={<Power className="w-3.5 h-3.5" />} label="重启" danger />
        <div className="flex-1" />

        <div className="flex items-center gap-3 mr-2">
          <div className="flex items-center gap-1.5 px-2 py-1 rounded-md bg-white/8 text-gray-300" style={{ fontSize: 11 }}>
            <Wifi className="w-3 h-3 text-green-400" />
            <span>{latency}ms</span>
          </div>
          <div className="flex items-center gap-1.5 px-2 py-1 rounded-md bg-white/8 text-gray-300" style={{ fontSize: 11 }}>
            <Monitor className="w-3 h-3 text-blue-400" />
            <span>{fpsLabel}</span>
          </div>
          <div className="px-2 py-1 rounded-md bg-white/8 text-gray-300" style={{ fontSize: 11 }}>
            {formatTime(elapsed)}
          </div>
        </div>

        <button
          onClick={() => void handleDisconnect()}
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-red-500/20 text-red-400 hover:bg-red-500/30 transition-colors"
          style={{ fontSize: 11 }}
        >
          <Power className="w-3 h-3" />
          断开
        </button>
      </div>

      {/* Screen */}
      <div className="flex-1 relative overflow-hidden cursor-crosshair select-none">
        <img
          src="https://images.unsplash.com/photo-1610529027273-97aa4ba7188c?crop=entropy&cs=tinysrgb&fit=max&fm=jpg&ixid=M3w3Nzg4Nzd8MHwxfHNlYXJjaHwxfHx3aW5kb3dzJTIwZGVza3RvcCUyMHdhbGxwYXBlciUyMGxhbmRzY2FwZXxlbnwxfHx8fDE3NzI2MjE0NTB8MA&ixlib=rb-4.1.0&q=80&w=1080"
          alt="Remote desktop"
          className="w-full h-full object-cover opacity-90"
          draggable={false}
        />
        <div className="absolute inset-0 bg-[#070b14]" />
        <div className="absolute inset-0 flex items-center justify-center px-6">
          <div className="w-full max-w-3xl rounded-xl border border-white/10 bg-white/[0.03] p-5 text-gray-200 shadow-2xl">
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 pb-4">
              <div className="min-w-0">
                <div className="flex items-center gap-2 text-sm font-semibold text-white">
                  <Monitor className="h-4 w-4 text-blue-300" />
                  Native remote window active
                </div>
                <div className="mt-1 truncate text-xs text-gray-400">
                  {remoteWindowLabel ?? activeSessionId ?? "session pending"}
                </div>
              </div>
              <button
                onClick={() => activeSessionId && navigate(`/session/${activeSessionId}`)}
                disabled={!activeSessionId}
                className="rounded-md bg-blue-500/20 px-3 py-1.5 text-xs text-blue-100 hover:bg-blue-500/30 disabled:cursor-not-allowed disabled:opacity-50"
              >
                Open session view
              </button>
            </div>
            <div className="grid grid-cols-2 gap-3 pt-4 md:grid-cols-5">
              <StatusPanel label="State" value={sessionStateLabel} />
              <StatusPanel label="FPS" value={fpsLabel} />
              <StatusPanel label="Size" value={frameSizeLabel} />
              <StatusPanel label="Bitrate" value={bitrateLabel} />
              <StatusPanel label="Frames" value={`${decodedFrames}`} />
            </div>
            {connectionError ? (
              <div className="mt-4 flex items-start gap-2 rounded-lg border border-red-500/20 bg-red-500/10 px-3 py-2 text-xs text-red-200">
                <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <span>{connectionError}</span>
              </div>
            ) : null}
          </div>
        </div>
        <div className="absolute top-3 right-3 flex items-center gap-2 px-2.5 py-1.5 rounded-lg bg-black/60 backdrop-blur-sm border border-white/10 text-gray-300" style={{ fontSize: 11 }}>
          <div className="w-1.5 h-1.5 rounded-full bg-green-400 animate-pulse" />
          {sessionStateLabel}
        </div>
        <div className="absolute bottom-3 left-3 px-2.5 py-1.5 rounded-lg bg-black/60 backdrop-blur-sm border border-white/10 text-gray-400" style={{ fontSize: 11 }}>
          {device.name} · {device.os} · 1920×1080
        </div>
      </div>

      {/* Status bar */}
      <div className="flex items-center justify-between px-4 py-1.5 bg-[#232340] border-t border-white/10 shrink-0">
        <div className="flex items-center gap-4">
          <StatusItem label="Size" value={frameSizeLabel} />
          <StatusItem label="FPS" value={fpsLabel} />
          <StatusItem label="Bitrate" value={bitrateLabel} />
        </div>
        <div className="hidden">
          <StatusItem label="分辨率" value="1920×1080" />
          <StatusItem label="帧率" value="60 fps" />
          <StatusItem label="带宽" value="4.2 MB/s" />
        </div>
        <div className="flex items-center gap-1 text-green-400" style={{ fontSize: 11 }}>
          <Lock className="w-3 h-3" />
          TLS 1.3 加密
        </div>
      </div>
    </div>
  );
}

/* ======================== File Transfer Tab ======================== */
type FileItem = { name: string; type: "folder" | "file"; size: string; modified: string; fileKind: string };

// Helper to get file system for a device
function getDeviceFileSystems(deviceId: string, devices: Device[]): FileItem[] {
  const dev = devices.find(d => d.id === deviceId);
  if (dev?.id === "1") return allRemoteFiles;
  return [
    { name: "Documents", type: "folder", size: "—", modified: "2026-03-02", fileKind: "文件夹" },
    { name: "Photos", type: "folder", size: "—", modified: "2026-03-01", fileKind: "文件夹" },
    { name: "Downloads", type: "folder", size: "—", modified: "2026-03-03", fileKind: "文件夹" },
    { name: "workspace.code", type: "file", size: "1.2 KB", modified: "2026-03-03", fileKind: "Code 文件" },
    { name: "readme.md", type: "file", size: "8 KB", modified: "2026-02-28", fileKind: "Markdown" },
    { name: "deploy.sh", type: "file", size: "2 KB", modified: "2026-03-01", fileKind: "Shell 脚本" },
  ];
}

function FilePane({
  deviceId, side, otherDeviceName, isDark, onSendToOther, dragOver, devices,
}: {
  deviceId: string; side: "left" | "right"; otherDeviceName: string | null; isDark: boolean;
  onSendToOther: (fileNames: string[]) => void; dragOver: boolean; devices: Device[];
}) {
  const dev = devices.find(d => d.id === deviceId);
  const devName = dev?.name ?? "未知设备";
  const contextMenuRef = useRef<HTMLDivElement>(null);
  const [currentPath, setCurrentPath] = useState<string[]>([devName, "Users", "Admin"]);
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(new Set());
  const [viewMode, setViewMode] = useState<"list" | "grid">("list");
  const [searchQuery, setSearchQuery] = useState("");
  const [contextMenuState, setContextMenuState] = useState<{ x: number; y: number; fileName: string; fileType: string } | null>(null);

  const files = (deviceId === "1" ? allRemoteFiles : getDeviceFileSystems(deviceId, devices)).filter(f =>
    searchQuery ? f.name.toLowerCase().includes(searchQuery.toLowerCase()) : true
  );

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (contextMenuRef.current && !contextMenuRef.current.contains(e.target as Node)) setContextMenuState(null);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, []);

  const handleContextMenu = (e: React.MouseEvent, fileName: string, fileType: string) => { e.preventDefault(); setContextMenuState({ x: e.clientX, y: e.clientY, fileName, fileType }); };
  const handleFileClick = (e: React.MouseEvent, fileName: string) => {
    if (e.ctrlKey || e.metaKey) {
      setSelectedFiles(prev => { const n = new Set(prev); if (n.has(fileName)) n.delete(fileName); else n.add(fileName); return n; });
    } else setSelectedFiles(new Set([fileName]));
  };
  const handleDoubleClick = (f: FileItem) => { if (f.type === "folder") { setCurrentPath(p => [...p, f.name]); setSelectedFiles(new Set()); } };
  const navigateUp = () => { if (currentPath.length > 1) { setCurrentPath(p => p.slice(0, -1)); setSelectedFiles(new Set()); } };
  const navigateHome = () => { setCurrentPath([devName, "Users", "Admin"]); setSelectedFiles(new Set()); };
  const navigateTo = (i: number) => { setCurrentPath(p => p.slice(0, i + 1)); setSelectedFiles(new Set()); };

  const handleDragStart = (e: React.DragEvent, fileName: string) => {
    const dragFiles = selectedFiles.has(fileName) ? Array.from(selectedFiles) : [fileName];
    e.dataTransfer.setData("fileTransfer", JSON.stringify({ files: dragFiles, fromSide: side, fromDeviceId: deviceId }));
    e.dataTransfer.effectAllowed = "copy";
  };

  const handlePaneDrop = (e: React.DragEvent) => {
    e.preventDefault();
    try {
      const parsed = JSON.parse(e.dataTransfer.getData("fileTransfer"));
      if (parsed.fromSide !== side) console.log(`Transfer ${parsed.files.join(", ")} → ${devName}`);
    } catch {}
  };

  const DevIcon = dev?.icon ?? Monitor;
  const getFileIcon = (f: FileItem) => {
    if (f.type === "folder") return <Folder className="w-4 h-4 text-yellow-500 shrink-0" />;
    if (f.name.endsWith(".png") || f.name.endsWith(".jpg")) return <Image className="w-4 h-4 text-green-500 shrink-0" />;
    if (f.name.endsWith(".pdf")) return <FileText className="w-4 h-4 text-red-500 shrink-0" />;
    if (f.name.endsWith(".mp3") || f.name.endsWith(".wav")) return <Music className="w-4 h-4 text-purple-500 shrink-0" />;
    return <File className={`w-4 h-4 shrink-0 ${isDark ? "text-gray-500" : "text-gray-400"}`} />;
  };

  return (
    <div className={`flex-1 flex flex-col min-w-0 relative ${dragOver ? (isDark ? "ring-2 ring-inset ring-blue-500/50" : "ring-2 ring-inset ring-blue-400/50") : ""}`}
      onDragOver={(e) => { e.preventDefault(); e.dataTransfer.dropEffect = "copy"; }} onDrop={handlePaneDrop}>
      {dragOver && (
        <div className="absolute inset-0 z-10 flex items-center justify-center pointer-events-none bg-blue-500/5">
          <div className={`px-4 py-2 rounded-lg border-2 border-dashed ${isDark ? "border-blue-500/40 bg-[#1e1e1e]/90 text-blue-400" : "border-blue-400/40 bg-white/90 text-blue-600"}`} style={{ fontSize: 13 }}>
            <Download style={{ width: 16, height: 16, display: "inline", marginRight: 6, verticalAlign: -3 }} />拖放到此处传输
          </div>
        </div>
      )}
      {/* Toolbar */}
      <div className={`flex items-center gap-1.5 px-2 py-1 border-b shrink-0 ${isDark ? "bg-[#232323] border-gray-700" : "bg-white border-gray-200"}`}>
        <div className={`flex items-center gap-1.5 px-2 py-0.5 rounded-md mr-1 ${isDark ? "bg-[#2a2a2a]" : "bg-gray-50"}`}>
          <DevIcon style={{ width: 12, height: 12 }} className={isDark ? "text-gray-400" : "text-gray-500"} />
          <span className={isDark ? "text-gray-300" : "text-gray-600"} style={{ fontSize: 11 }}>{devName}</span>
          <div className={`w-1.5 h-1.5 rounded-full ${dev?.status === "online" ? "bg-green-500" : "bg-gray-400"}`} />
        </div>
        <div className={`w-px h-4 ${isDark ? "bg-gray-700" : "bg-gray-200"}`} />
        <button onClick={navigateUp} className={`p-1 rounded-md transition-colors ${isDark ? "text-gray-400 hover:bg-gray-700 hover:text-gray-200" : "text-gray-500 hover:bg-gray-100 hover:text-gray-700"} ${currentPath.length <= 1 ? "opacity-40 pointer-events-none" : ""}`} title="上级目录">
          <ArrowUp style={{ width: 13, height: 13 }} />
        </button>
        <button onClick={navigateHome} className={`p-1 rounded-md transition-colors ${isDark ? "text-gray-400 hover:bg-gray-700 hover:text-gray-200" : "text-gray-500 hover:bg-gray-100 hover:text-gray-700"}`} title="主目录">
          <Home style={{ width: 13, height: 13 }} />
        </button>
        <button className={`p-1 rounded-md transition-colors ${isDark ? "text-gray-400 hover:bg-gray-700 hover:text-gray-200" : "text-gray-500 hover:bg-gray-100 hover:text-gray-700"}`} title="刷新">
          <RefreshCw style={{ width: 12, height: 12 }} />
        </button>
        <div className={`flex-1 flex items-center gap-0.5 px-2 py-0.5 rounded-md min-w-0 ${isDark ? "bg-[#2a2a2a] border border-gray-700" : "bg-gray-50 border border-gray-200"}`}>
          {currentPath.map((seg, i) => (
            <span key={i} className="flex items-center gap-0.5 shrink-0">
              {i > 0 && <ChevronRight style={{ width: 9, height: 9 }} className={isDark ? "text-gray-600" : "text-gray-300"} />}
              <button onClick={() => navigateTo(i)} className={`px-0.5 rounded transition-colors truncate ${isDark ? "text-gray-300 hover:text-blue-400 hover:bg-gray-700" : "text-gray-600 hover:text-blue-600 hover:bg-gray-100"}`} style={{ fontSize: 10, maxWidth: 90 }}>{seg}</button>
            </span>
          ))}
        </div>
        <div className={`flex items-center gap-1 px-2 py-0.5 rounded-md w-32 ${isDark ? "bg-[#2a2a2a] border border-gray-700" : "bg-gray-50 border border-gray-200"}`}>
          <Search style={{ width: 11, height: 11 }} className={isDark ? "text-gray-500" : "text-gray-400"} />
          <input value={searchQuery} onChange={(e) => setSearchQuery(e.target.value)} placeholder="搜索..." className={`bg-transparent outline-none flex-1 min-w-0 placeholder-gray-500 ${isDark ? "text-gray-200" : "text-gray-700"}`} style={{ fontSize: 10 }} />
        </div>
        <div className={`flex items-center rounded-md overflow-hidden border ${isDark ? "border-gray-700" : "border-gray-200"}`}>
          <button onClick={() => setViewMode("list")} className={`p-1 transition-colors ${viewMode === "list" ? (isDark ? "bg-gray-700 text-gray-200" : "bg-gray-100 text-gray-700") : (isDark ? "text-gray-500 hover:text-gray-300" : "text-gray-400 hover:text-gray-600")}`}>
            <List style={{ width: 12, height: 12 }} />
          </button>
          <button onClick={() => setViewMode("grid")} className={`p-1 transition-colors ${viewMode === "grid" ? (isDark ? "bg-gray-700 text-gray-200" : "bg-gray-100 text-gray-700") : (isDark ? "text-gray-500 hover:text-gray-300" : "text-gray-400 hover:text-gray-600")}`}>
            <LayoutGrid style={{ width: 12, height: 12 }} />
          </button>
        </div>
      </div>
      {viewMode === "list" && (
        <div className={`flex items-center px-3 py-0.5 border-b shrink-0 ${isDark ? "border-gray-700/60 bg-[#232323]" : "border-gray-100 bg-white"}`}>
          <span className={`flex-1 ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 10 }}>名称</span>
          <span className={`w-24 text-right ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 10 }}>修改日期</span>
          <span className={`w-20 text-right ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 10 }}>类型</span>
          <span className={`w-16 text-right ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 10 }}>大小</span>
        </div>
      )}
      <div
        className={`flex-1 overflow-y-auto ${viewMode === "grid" ? "p-2" : ""} ${isDark ? "bg-[#1e1e1e]" : "bg-white"}`}
        onClick={(e) => { if (e.target === e.currentTarget) setSelectedFiles(new Set()); }}
        onContextMenu={(e) => { if (e.target === e.currentTarget) { e.preventDefault(); setContextMenuState({ x: e.clientX, y: e.clientY, fileName: "", fileType: "background" }); } }}
      >
        {viewMode === "list" ? (
          <div>
            {files.map((f) => {
              const isSel = selectedFiles.has(f.name);
              return (
                <div key={f.name} draggable onDragStart={(e) => handleDragStart(e, f.name)}
                  onClick={(e) => handleFileClick(e, f.name)} onDoubleClick={() => handleDoubleClick(f)}
                  onContextMenu={(e) => handleContextMenu(e, f.name, f.type)}
                  className={`flex items-center px-3 py-1 cursor-default transition-colors ${isSel ? (isDark ? "bg-blue-900/30" : "bg-blue-50") : (isDark ? "hover:bg-[#252525]" : "hover:bg-gray-50/80")}`}>
                  <div className="flex items-center gap-2 flex-1 min-w-0">
                    {getFileIcon(f)}
                    <span className={`truncate ${isSel ? (isDark ? "text-blue-300" : "text-blue-700") : (isDark ? "text-gray-300" : "text-gray-700")}`} style={{ fontSize: 11 }}>{f.name}</span>
                  </div>
                  <span className={`w-24 text-right shrink-0 ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 10 }}>{f.modified}</span>
                  <span className={`w-20 text-right shrink-0 ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 10 }}>{f.fileKind}</span>
                  <span className={`w-16 text-right shrink-0 ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 10 }}>{f.size}</span>
                </div>
              );
            })}
          </div>
        ) : (
          <div className="grid grid-cols-4 gap-1.5">
            {files.map((f) => {
              const isSel = selectedFiles.has(f.name);
              return (
                <div key={f.name} draggable onDragStart={(e) => handleDragStart(e, f.name)}
                  onClick={(e) => handleFileClick(e, f.name)} onDoubleClick={() => handleDoubleClick(f)}
                  onContextMenu={(e) => handleContextMenu(e, f.name, f.type)}
                  className={`flex flex-col items-center gap-1 p-2.5 rounded-lg cursor-default transition-colors ${isSel ? (isDark ? "bg-blue-900/30" : "bg-blue-50") : (isDark ? "hover:bg-[#252525]" : "hover:bg-gray-50")}`}>
                  {f.type === "folder" ? <Folder className="w-7 h-7 text-yellow-500" />
                    : f.name.endsWith(".png") || f.name.endsWith(".jpg") ? <Image className="w-7 h-7 text-green-500" />
                    : f.name.endsWith(".pdf") ? <FileText className="w-7 h-7 text-red-500" />
                    : <File className={`w-7 h-7 ${isDark ? "text-gray-500" : "text-gray-400"}`} />}
                  <span className={`text-center truncate w-full ${isSel ? (isDark ? "text-blue-300" : "text-blue-700") : (isDark ? "text-gray-300" : "text-gray-700")}`} style={{ fontSize: 10 }}>{f.name}</span>
                </div>
              );
            })}
          </div>
        )}
      </div>
      <div className={`flex items-center justify-between px-3 py-0.5 border-t shrink-0 ${isDark ? "bg-[#232323] border-gray-700" : "bg-white border-gray-200"}`}>
        <div className="flex items-center gap-2">
          <span className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 9 }}>{files.length} 个项目</span>
          {selectedFiles.size > 0 && <span className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 9 }}>已选择 {selectedFiles.size} 项</span>}
        </div>
        <div className="flex items-center gap-1">
          <Lock style={{ width: 8, height: 8 }} className="text-green-500" />
          <span className="text-green-600" style={{ fontSize: 9 }}>E2E</span>
        </div>
      </div>
      {contextMenuState && (
        <div ref={contextMenuRef} className={`fixed z-50 rounded-lg border shadow-lg py-1 min-w-[180px] ${isDark ? "bg-[#2a2a2a] border-gray-700" : "bg-white border-gray-200"}`} style={{ left: contextMenuState.x, top: contextMenuState.y }}>
          {contextMenuState.fileType !== "background" ? (
            <>
              {contextMenuState.fileType === "folder" && (
                <CtxItem icon={<FolderOpen style={{ width: 13, height: 13 }} />} label="打开" onClick={() => { handleDoubleClick({ name: contextMenuState.fileName, type: "folder", size: "", modified: "", fileKind: "" }); setContextMenuState(null); }} isDark={isDark} />
              )}
              <CtxItem icon={<Download style={{ width: 13, height: 13 }} />} label="下载到本地" onClick={() => setContextMenuState(null)} isDark={isDark} />
              {otherDeviceName && (
                <CtxItem icon={<Send style={{ width: 13, height: 13 }} />} label={`发送到 ${otherDeviceName}`}
                  onClick={() => { onSendToOther([contextMenuState.fileName]); setContextMenuState(null); }} isDark={isDark} />
              )}
              <div className={`h-px mx-2 my-1 ${isDark ? "bg-gray-700" : "bg-gray-200"}`} />
              <CtxItem icon={<Scissors style={{ width: 13, height: 13 }} />} label="剪切" onClick={() => setContextMenuState(null)} isDark={isDark} />
              <CtxItem icon={<Copy style={{ width: 13, height: 13 }} />} label="复制" onClick={() => setContextMenuState(null)} isDark={isDark} />
              <CtxItem icon={<Edit3 style={{ width: 13, height: 13 }} />} label="重命名" onClick={() => setContextMenuState(null)} isDark={isDark} />
              <CtxItem icon={<Trash2 style={{ width: 13, height: 13 }} />} label="删除" onClick={() => setContextMenuState(null)} isDark={isDark} danger />
              <div className={`h-px mx-2 my-1 ${isDark ? "bg-gray-700" : "bg-gray-200"}`} />
              <CtxItem icon={<Info style={{ width: 13, height: 13 }} />} label="属性" onClick={() => setContextMenuState(null)} isDark={isDark} />
            </>
          ) : (
            <>
              <CtxItem icon={<Upload style={{ width: 13, height: 13 }} />} label="上传文件到此处" onClick={() => setContextMenuState(null)} isDark={isDark} />
              <CtxItem icon={<Folder style={{ width: 13, height: 13 }} />} label="新建文件夹" onClick={() => setContextMenuState(null)} isDark={isDark} />
              <div className={`h-px mx-2 my-1 ${isDark ? "bg-gray-700" : "bg-gray-200"}`} />
              <CtxItem icon={<ClipboardPaste style={{ width: 13, height: 13 }} />} label="粘贴" onClick={() => setContextMenuState(null)} isDark={isDark} />
              <CtxItem icon={<RefreshCw style={{ width: 13, height: 13 }} />} label="刷新" onClick={() => setContextMenuState(null)} isDark={isDark} />
            </>
          )}
        </div>
      )}
    </div>
  );
}

function FilesTab({ device, devices }: { device: Device; devices: Device[] }) {
  const { isDark } = useTheme();
  const isOnline = device.status === "online";

  const [leftDeviceId, setLeftDeviceId] = useState(device.id);
  const [rightDeviceId, setRightDeviceId] = useState<string | null>(null);
  const [showAddMenu, setShowAddMenu] = useState(false);
  const [addMenuSide, setAddMenuSide] = useState<"left" | "right">("right");
  const addMenuRef = useRef<HTMLDivElement>(null);
  const addBtnRef = useRef<HTMLButtonElement>(null);
  const [dragOverSide, setDragOverSide] = useState<"left" | "right" | null>(null);

  const onlineDevices = devices.filter(d => d.status === "online");
  const leftDevice = devices.find(d => d.id === leftDeviceId);
  const rightDevice = rightDeviceId ? devices.find(d => d.id === rightDeviceId) : null;

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (addMenuRef.current && !addMenuRef.current.contains(e.target as Node) && addBtnRef.current && !addBtnRef.current.contains(e.target as Node)) setShowAddMenu(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, []);

  const handleAddDevice = (dId: string) => {
    if (addMenuSide === "right") setRightDeviceId(dId);
    else setLeftDeviceId(dId);
    setShowAddMenu(false);
  };

  if (!isOnline) {
    return (
      <div className={`flex items-center justify-center h-full ${isDark ? "bg-[#1a1a1a]" : "bg-[#f0f2f5]"}`}>
        <div className="text-center">
          <WifiOff className={`w-12 h-12 mx-auto mb-3 ${isDark ? "text-gray-600" : "text-gray-300"}`} />
          <div className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 16 }}>设备离线，无法传输文件</div>
        </div>
      </div>
    );
  }

  return (
    <div className={`flex h-full overflow-hidden ${isDark ? "bg-[#1a1a1a]" : "bg-[#f0f2f5]"}`}>
      {/* Left pane */}
      <div
        className="flex-1 flex flex-col min-w-0"
        onDragOver={(e) => { e.preventDefault(); setDragOverSide("left"); }}
        onDragLeave={() => setDragOverSide(null)}
        onDrop={() => setDragOverSide(null)}
      >
        <FilePane
          deviceId={leftDeviceId}
          side="left"
          otherDeviceName={rightDevice?.name ?? null}
          isDark={isDark}
          onSendToOther={(fileNames) => console.log(`Send ${fileNames.join(", ")} to right pane`)}
          dragOver={dragOverSide === "left"}
          devices={devices}
        />
      </div>

      {/* Center divider with + button */}
      <div className={`relative w-8 shrink-0 flex flex-col items-center justify-center border-x ${isDark ? "bg-[#1a1a1a] border-gray-700/60" : "bg-[#f0f2f5] border-gray-200"}`}>
        <div className={`flex flex-col items-center gap-1 mb-2 ${isDark ? "text-gray-600" : "text-gray-300"}`}>
          <ChevronRight style={{ width: 12, height: 12 }} />
          <ChevronRight style={{ width: 12, height: 12, transform: "rotate(180deg)" }} />
        </div>

        <button
          ref={addBtnRef}
          onClick={() => { setAddMenuSide("right"); setShowAddMenu(!showAddMenu); }}
          className={`w-7 h-7 rounded-full flex items-center justify-center border transition-all ${isDark ? "bg-[#232323] border-gray-600 text-gray-400 hover:border-blue-500 hover:text-blue-400 hover:bg-blue-900/20" : "bg-white border-gray-300 text-gray-400 hover:border-blue-400 hover:text-blue-500 hover:bg-blue-50"} shadow-sm`}
          title="添加设备"
        >
          <Plus style={{ width: 13, height: 13 }} />
        </button>

        <div className={`flex flex-col items-center gap-1 mt-2 ${isDark ? "text-gray-600" : "text-gray-300"}`}>
          <ArrowRightLeft style={{ width: 12, height: 12 }} />
        </div>

        {/* Add device menu */}
        {showAddMenu && (
          <div ref={addMenuRef} className={`absolute z-50 top-1/2 left-full ml-2 -translate-y-1/2 rounded-xl border shadow-xl py-2 min-w-[220px] ${isDark ? "bg-[#2a2a2a] border-gray-700" : "bg-white border-gray-200"}`}>
            <div className={`px-3 py-1.5 ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 10 }}>选择设备</div>
            {onlineDevices.map((d) => {
              const DIcon = d.icon;
              const isActive = d.id === leftDeviceId || d.id === rightDeviceId;
              return (
                <button
                  key={d.id}
                  onClick={() => handleAddDevice(d.id)}
                  disabled={isActive}
                  className={`w-full flex items-center gap-2.5 px-3 py-2 transition-colors ${isActive ? (isDark ? "text-gray-600 cursor-not-allowed" : "text-gray-300 cursor-not-allowed") : (isDark ? "text-gray-300 hover:bg-gray-700" : "text-gray-700 hover:bg-gray-50")}`}
                  style={{ fontSize: 12 }}
                >
                  <div className={`w-6 h-6 rounded-md flex items-center justify-center ${isDark ? "bg-gray-700" : "bg-gray-100"}`}>
                    <DIcon style={{ width: 13, height: 13 }} />
                  </div>
                  <div className="flex-1 text-left min-w-0">
                    <div className="truncate">{d.name}</div>
                    <div className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 10 }}>{d.os} · {d.ip}</div>
                  </div>
                  {isActive && (
                    <span className={`px-1.5 py-0.5 rounded ${isDark ? "bg-gray-700 text-gray-500" : "bg-gray-100 text-gray-400"}`} style={{ fontSize: 9 }}>已添加</span>
                  )}
                  <div className="w-1.5 h-1.5 rounded-full bg-green-500 shrink-0" />
                </button>
              );
            })}
            {onlineDevices.length === 0 && (
              <div className={`px-3 py-4 text-center ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 12 }}>没有在线设备</div>
            )}
          </div>
        )}
      </div>

      {/* Right pane */}
      <div
        className="flex-1 flex flex-col min-w-0"
        onDragOver={(e) => { e.preventDefault(); setDragOverSide("right"); }}
        onDragLeave={() => setDragOverSide(null)}
        onDrop={() => setDragOverSide(null)}
      >
        {rightDeviceId ? (
          <FilePane
            deviceId={rightDeviceId}
            side="right"
            otherDeviceName={leftDevice?.name ?? null}
            isDark={isDark}
            onSendToOther={(fileNames) => console.log(`Send ${fileNames.join(", ")} to left pane`)}
            dragOver={dragOverSide === "right"}
            devices={devices}
          />
        ) : (
          <div className={`flex-1 flex flex-col items-center justify-center ${isDark ? "bg-[#1e1e1e]" : "bg-white"}`}>
            <div className={`w-14 h-14 rounded-2xl flex items-center justify-center mb-4 ${isDark ? "bg-gray-800" : "bg-gray-50"}`}>
              <Monitor className={`w-7 h-7 ${isDark ? "text-gray-600" : "text-gray-300"}`} />
            </div>
            <div className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 14 }}>选择设备以开始传输</div>
            <div className={`mt-1 ${isDark ? "text-gray-600" : "text-gray-300"}`} style={{ fontSize: 12 }}>点击中间的 + 号添加设备，或从侧边栏拖入</div>
            <button
              onClick={() => { setAddMenuSide("right"); setShowAddMenu(true); }}
              className={`mt-5 flex items-center gap-2 px-4 py-2 rounded-lg border transition-colors ${isDark ? "bg-[#232323] border-gray-600 text-gray-300 hover:border-blue-500 hover:text-blue-400" : "bg-gray-50 border-gray-200 text-gray-600 hover:border-blue-400 hover:text-blue-500"}`}
              style={{ fontSize: 12 }}
            >
              <Plus style={{ width: 14, height: 14 }} />
              添加设备
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

function CtxItem({ icon, label, onClick, isDark, danger }: { icon: React.ReactNode; label: string; onClick: () => void; isDark: boolean; danger?: boolean }) {
  return (
    <button onClick={onClick} className={`w-full flex items-center gap-2.5 px-3 py-1.5 transition-colors ${danger ? (isDark ? "text-red-400 hover:bg-red-900/30" : "text-red-600 hover:bg-red-50") : (isDark ? "text-gray-300 hover:bg-gray-700" : "text-gray-700 hover:bg-gray-50")}`} style={{ fontSize: 12 }}>
      <span className={danger ? "" : isDark ? "text-gray-500" : "text-gray-400"}>{icon}</span>{label}
    </button>
  );
}

/* ======================== Remote Apps Tab ======================== */
function AppsTab({ device }: { device: Device }) {
  const { isDark } = useTheme();
  const navigate = useNavigate();
  const [catalog, setCatalog] = useState<RemoteApplicationCatalogResult | null>(null);
  const [sourcesLoading, setSourcesLoading] = useState(false);
  const [openingSourceId, setOpeningSourceId] = useState<string | null>(null);
  const [openingDesktop, setOpeningDesktop] = useState(false);
  const [activeSelection, setActiveSelection] =
    useState<CaptureSourceSelection | null>(null);
  const [appsError, setAppsError] = useState<string | null>(null);
  const appSessionIdRef = useRef<string | null>(null);
  const sessionHandedOffRef = useRef(false);
  const isOnline = device.status === "online";
  const desktopRuntime = isTauriRuntime();
  const isLanP2PRemote = device.p2pAvailable && !device.isLocal;
  const canUseRemoteApplications = desktopRuntime && isLanP2PRemote;

  useEffect(() => {
    return () => {
      const sessionId = appSessionIdRef.current;
      if (!sessionId || sessionHandedOffRef.current) return;
      void stopSession(sessionId).catch(() => undefined);
    };
  }, []);

  const loadRemoteApplications = useCallback(async () => {
    if (!canUseRemoteApplications || sourcesLoading) return;

    setSourcesLoading(true);
    setAppsError(null);
    try {
      const existingSessionId = appSessionIdRef.current;
      const nextCatalog = await prepareRemoteApplicationCatalogForDevice(
        device.deviceId,
        {
          sessionId: existingSessionId ?? undefined,
          sessionAlreadyStarted: Boolean(existingSessionId),
          transportKind: "quic",
          targetDeviceName: device.name,
          targetOs: device.os,
          targetIp: device.ip,
          lanP2P: true,
          includePreviews: false,
          limit: 48,
        }
      );
      appSessionIdRef.current = nextCatalog.sessionId;
      setCatalog(nextCatalog);
    } catch (error) {
      setAppsError(error instanceof Error ? error.message : String(error));
    } finally {
      setSourcesLoading(false);
    }
  }, [
    canUseRemoteApplications,
    device.deviceId,
    device.ip,
    device.name,
    device.os,
    sourcesLoading,
  ]);

  useEffect(() => {
    if (!isOnline || !canUseRemoteApplications || catalog || sourcesLoading || appsError) return;
    void loadRemoteApplications();
  }, [
    appsError,
    canUseRemoteApplications,
    catalog,
    isOnline,
    loadRemoteApplications,
    sourcesLoading,
  ]);

  const handleOpenDesktop = async () => {
    setOpeningDesktop(true);
    setAppsError(null);
    try {
      const result = await launchRemoteDisplayForDevice(device.deviceId, {
        transportKind: device.p2pAvailable ? "quic" : "webrtc",
        targetDeviceName: device.name,
        targetOs: device.os,
        targetIp: device.ip,
        lanP2P: isLanP2PRemote,
      });
      if (result.mode === "route") navigate(`/session/${result.sessionId}`);
    } catch (error) {
      setAppsError(error instanceof Error ? error.message : String(error));
    } finally {
      setOpeningDesktop(false);
    }
  };

  const handleOpenApplication = async (source: CaptureSource) => {
    setOpeningSourceId(source.id);
    setAppsError(null);
    try {
      let sessionId = appSessionIdRef.current;
      if (!sessionId) {
        const nextCatalog = await prepareRemoteApplicationCatalogForDevice(
          device.deviceId,
          {
            transportKind: "quic",
            targetDeviceName: device.name,
            targetOs: device.os,
            targetIp: device.ip,
            lanP2P: true,
            includePreviews: false,
            limit: 48,
          }
        );
        sessionId = nextCatalog.sessionId;
        appSessionIdRef.current = nextCatalog.sessionId;
        setCatalog(nextCatalog);
      }

      const result = await launchRemoteApplicationForDevice(device.deviceId, source.id, {
        sessionId,
        sessionAlreadyStarted: true,
        transportKind: "quic",
        targetDeviceName: device.name,
        targetOs: device.os,
        targetIp: device.ip,
        lanP2P: true,
      });
      sessionHandedOffRef.current = true;
      setActiveSelection(result.captureSourceSelection ?? null);
      if (result.mode === "route") navigate(`/session/${result.sessionId}`);
    } catch (error) {
      setAppsError(error instanceof Error ? error.message : String(error));
    } finally {
      setOpeningSourceId(null);
    }
  };

  if (!isOnline) {
    return (
      <div className={`flex items-center justify-center h-full ${isDark ? "bg-[#1a1a1a]" : "bg-[#f0f2f5]"}`}>
        <div className="text-center">
          <WifiOff className={`w-12 h-12 mx-auto mb-3 ${isDark ? "text-gray-600" : "text-gray-300"}`} />
          <div className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 16 }}>设备离线，无法启动远程应用</div>
        </div>
      </div>
    );
  }

  const unavailableReason = !desktopRuntime
    ? "远程应用需要桌面端运行"
    : device.isLocal
      ? "本机设备请使用本地测试工作台"
      : !device.p2pAvailable
        ? "当前设备未建立 LAN P2P 通道"
        : null;
  const remoteWindows = catalog?.windows ?? [];
  const displaySources = catalog?.displays ?? [];

  if (!canUseRemoteApplications) {
    return (
      <div className={`flex items-center justify-center h-full p-6 ${isDark ? "bg-[#1a1a1a]" : "bg-[#f0f2f5]"}`}>
        <div className={`w-full max-w-[560px] rounded-xl border p-6 shadow-sm ${isDark ? "bg-[#202020] border-gray-700" : "bg-white border-gray-200"}`}>
          <div className={`w-12 h-12 rounded-2xl flex items-center justify-center mb-4 ${isDark ? "bg-cyan-900/30" : "bg-cyan-50"}`}>
            <AppWindow className="w-6 h-6 text-cyan-500" />
          </div>
          <div className={isDark ? "text-gray-100" : "text-gray-900"} style={{ fontSize: 18 }}>远程应用不可用</div>
          <div className={`mt-1 ${isDark ? "text-gray-500" : "text-gray-500"}`} style={{ fontSize: 13 }}>
            {unavailableReason}
          </div>
          {appsError && (
            <div className={`mt-4 rounded-lg border px-3 py-2 ${isDark ? "border-red-900/60 bg-red-950/20 text-red-300" : "border-red-100 bg-red-50 text-red-600"}`} style={{ fontSize: 12 }}>
              {appsError}
            </div>
          )}
          <div className="mt-5 flex items-center gap-2">
            <button
              onClick={() => void handleOpenDesktop()}
              disabled={!desktopRuntime || openingDesktop}
              className="inline-flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-white transition-colors hover:bg-blue-500 disabled:cursor-not-allowed disabled:opacity-60"
              style={{ fontSize: 13 }}
            >
              {openingDesktop ? <Loader2 className="h-4 w-4 animate-spin" /> : <Monitor className="h-4 w-4" />}
              打开远程桌面
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className={`h-full overflow-y-auto p-5 ${isDark ? "bg-[#1a1a1a]" : "bg-[#f0f2f5]"}`}>
      <div className="mx-auto max-w-6xl">
        <div className={`mb-5 rounded-xl border p-4 shadow-sm ${isDark ? "bg-[#202020] border-gray-700" : "bg-white border-gray-200"}`}>
          <div className="flex flex-wrap items-center gap-3">
            <div className={`flex h-11 w-11 items-center justify-center rounded-xl ${isDark ? "bg-cyan-900/30" : "bg-cyan-50"}`}>
              <AppWindow className="h-5 w-5 text-cyan-500" />
            </div>
            <div className="min-w-0 flex-1">
              <div className={`font-semibold ${isDark ? "text-gray-100" : "text-gray-900"}`} style={{ fontSize: 16 }}>远程应用</div>
              <div className={`mt-0.5 truncate ${isDark ? "text-gray-500" : "text-gray-500"}`} style={{ fontSize: 12 }}>
                {device.name} · {device.ip} · LAN QUIC 窗口流
              </div>
            </div>
            <div className={`rounded-lg border px-3 py-1.5 ${isDark ? "border-gray-700 bg-[#181818] text-gray-400" : "border-gray-200 bg-gray-50 text-gray-600"}`} style={{ fontSize: 12 }}>
              {catalog ? `${remoteWindows.length} 个窗口 / ${displaySources.length} 个屏幕` : "等待枚举"}
            </div>
            <button
              onClick={() => void loadRemoteApplications()}
              disabled={sourcesLoading}
              className={`inline-flex items-center gap-2 rounded-lg border px-3 py-2 transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${isDark ? "border-gray-700 bg-[#1b1b1b] text-gray-300 hover:border-cyan-600" : "border-gray-200 bg-white text-gray-700 hover:border-cyan-300"}`}
              style={{ fontSize: 12 }}
            >
              {sourcesLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
              刷新
            </button>
          </div>
        </div>

        {activeSelection && (
          <div className={`mb-5 flex items-center gap-3 rounded-xl border p-3 ${isDark ? "border-green-900/60 bg-green-950/20" : "border-green-100 bg-green-50"}`}>
            <div className="h-2 w-2 rounded-full bg-green-500" />
            <div className="min-w-0 flex-1">
              <div className={isDark ? "text-green-300" : "text-green-700"} style={{ fontSize: 13 }}>
                已打开 {remoteCaptureSourceTitle(activeSelection.source)}
              </div>
              <div className={isDark ? "text-green-500" : "text-green-600"} style={{ fontSize: 11 }}>
                {activeSelection.session_id} · {remoteCaptureSourceMeta(activeSelection.source)}
              </div>
            </div>
          </div>
        )}

        {appsError && (
          <div className={`mb-5 flex items-start gap-3 rounded-xl border p-3 ${isDark ? "border-red-900/60 bg-red-950/20" : "border-red-100 bg-red-50"}`}>
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-red-500" />
            <div className={isDark ? "text-red-300" : "text-red-600"} style={{ fontSize: 12 }}>{appsError}</div>
          </div>
        )}

        {sourcesLoading && !catalog && (
          <div className={`rounded-xl border p-10 text-center ${isDark ? "border-gray-700 bg-[#202020]" : "border-gray-200 bg-white"}`}>
            <Loader2 className="mx-auto mb-3 h-7 w-7 animate-spin text-cyan-500" />
            <div className={isDark ? "text-gray-300" : "text-gray-700"} style={{ fontSize: 14 }}>正在枚举远端窗口</div>
          </div>
        )}

        {!sourcesLoading && catalog && remoteWindows.length === 0 && (
          <div className={`rounded-xl border p-6 text-center ${isDark ? "border-gray-700 bg-[#202020]" : "border-gray-200 bg-white"}`}>
            <AppWindow className={`mx-auto mb-3 h-10 w-10 ${isDark ? "text-gray-600" : "text-gray-300"}`} />
            <div className={isDark ? "text-gray-300" : "text-gray-700"} style={{ fontSize: 15 }}>未发现可独立捕获的窗口</div>
            <div className={`mt-1 ${isDark ? "text-gray-500" : "text-gray-500"}`} style={{ fontSize: 12 }}>
              已发现 {catalog.sources.length} 个采集源，可先打开远程桌面或在远端启动目标应用后刷新。
            </div>
            <button
              onClick={() => void handleOpenDesktop()}
              disabled={openingDesktop}
              className="mt-5 inline-flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-white transition-colors hover:bg-blue-500 disabled:cursor-not-allowed disabled:opacity-60"
              style={{ fontSize: 13 }}
            >
              {openingDesktop ? <Loader2 className="h-4 w-4 animate-spin" /> : <Monitor className="h-4 w-4" />}
              打开远程桌面
            </button>
          </div>
        )}

        {remoteWindows.length > 0 && (
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
            {remoteWindows.map((source) => {
              const SourceIcon = remoteCaptureSourceIcon(source);
              const opening = openingSourceId === source.id;
              return (
                <div
                  key={source.id}
                  className={`group overflow-hidden rounded-xl border shadow-sm transition-colors ${isDark ? "border-gray-700 bg-[#202020] hover:border-cyan-700" : "border-gray-200 bg-white hover:border-cyan-300"}`}
                >
                  <div className={`flex h-28 items-center justify-center border-b ${isDark ? "border-gray-700 bg-[#151515]" : "border-gray-100 bg-gray-50"}`}>
                    {source.preview_data_url ? (
                      <img
                        src={source.preview_data_url}
                        alt={remoteCaptureSourceTitle(source)}
                        className="h-full w-full object-cover"
                        draggable={false}
                      />
                    ) : (
                      <div className={`flex h-14 w-14 items-center justify-center rounded-2xl ${remoteCaptureSourceAccent(source)}`}>
                        <SourceIcon className="h-7 w-7 text-white" />
                      </div>
                    )}
                  </div>
                  <div className="p-4">
                    <div className="flex items-start gap-3">
                      <div className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-lg ${remoteCaptureSourceAccent(source)}`}>
                        <SourceIcon className="h-4 w-4 text-white" />
                      </div>
                      <div className="min-w-0 flex-1">
                        <div className={`truncate font-medium ${isDark ? "text-gray-100" : "text-gray-900"}`} style={{ fontSize: 13 }}>
                          {remoteCaptureSourceTitle(source)}
                        </div>
                        <div className={`mt-0.5 truncate ${isDark ? "text-gray-500" : "text-gray-500"}`} style={{ fontSize: 11 }}>
                          {remoteCaptureSourceMeta(source)}
                        </div>
                      </div>
                    </div>
                    <button
                      onClick={() => void handleOpenApplication(source)}
                      disabled={openingSourceId !== null}
                      className="mt-4 inline-flex w-full items-center justify-center gap-2 rounded-lg bg-cyan-600 px-3 py-2 text-white transition-colors hover:bg-cyan-500 disabled:cursor-not-allowed disabled:opacity-60"
                      style={{ fontSize: 12 }}
                    >
                      {opening ? <Loader2 className="h-4 w-4 animate-spin" /> : <ExternalLink className="h-4 w-4" />}
                      {opening ? "正在打开" : "打开应用"}
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

function remoteCaptureSourceTitle(source: CaptureSource): string {
  return source.app_name?.trim() || source.title?.trim() || "Remote window";
}

function remoteCaptureSourceMeta(source: CaptureSource): string {
  const details = [
    source.title && source.title !== source.app_name ? source.title : null,
    remoteCaptureSourceResolution(source),
    source.process_id > 0 ? `PID ${source.process_id}` : null,
  ].filter(Boolean);
  return details.join(" · ") || remoteCaptureSourceKindLabel(source.source_kind);
}

function remoteCaptureSourceResolution(source: CaptureSource): string | null {
  if (source.width > 0 && source.height > 0) {
    return `${source.width}x${source.height}`;
  }
  return null;
}

function remoteCaptureSourceKindLabel(kind: string): string {
  if (kind === "window") return "窗口";
  if (kind === "display_shared") return "共享屏幕";
  if (kind === "display") return "屏幕";
  return kind;
}

function remoteCaptureSourceIcon(source: CaptureSource): typeof AppWindow {
  const text = `${source.app_name ?? ""} ${source.title ?? ""} ${source.class_name ?? ""}`.toLowerCase();
  if (text.includes("terminal") || text.includes("powershell") || text.includes("cmd")) return Terminal;
  if (text.includes("chrome") || text.includes("edge") || text.includes("firefox") || text.includes("browser")) return Globe;
  if (text.includes("code") || text.includes("visual studio") || text.includes("ide")) return Code;
  if (text.includes("powerpoint") || text.includes("presentation")) return Presentation;
  if (text.includes("excel") || text.includes("word") || text.includes("office") || text.includes("pdf")) return FileText;
  return AppWindow;
}

function remoteCaptureSourceAccent(source: CaptureSource): string {
  const text = `${source.app_name ?? ""} ${source.title ?? ""} ${source.class_name ?? ""}`.toLowerCase();
  if (text.includes("terminal") || text.includes("powershell") || text.includes("cmd")) return "bg-gray-700";
  if (text.includes("chrome") || text.includes("edge") || text.includes("firefox") || text.includes("browser")) return "bg-amber-500";
  if (text.includes("code") || text.includes("visual studio") || text.includes("ide")) return "bg-blue-600";
  if (text.includes("powerpoint") || text.includes("presentation")) return "bg-orange-600";
  if (text.includes("excel")) return "bg-green-600";
  if (text.includes("word") || text.includes("office") || text.includes("pdf")) return "bg-indigo-600";
  return "bg-cyan-600";
}

/* ======================== Performance Monitoring Footer ======================== */
function PerformanceFooter({ device }: { device: Device }) {
  const { isDark } = useTheme();
  const [cpu, setCpu] = useState(device.cpu ?? 0);
  const [ram, setRam] = useState(device.ram ?? 0);
  const [disk] = useState(device.disk ?? 0);
  const [netUp, setNetUp] = useState(2.4);
  const [netDown, setNetDown] = useState(8.7);

  useEffect(() => {
    const timer = setInterval(() => {
      setCpu((v) => Math.max(5, Math.min(95, v + Math.floor(Math.random() * 9) - 4)));
      setRam((v) => Math.max(30, Math.min(90, v + Math.floor(Math.random() * 5) - 2)));
      setNetUp((v) => Math.max(0.5, Math.min(12, +(v + (Math.random() * 2 - 1)).toFixed(1))));
      setNetDown((v) => Math.max(1, Math.min(25, +(v + (Math.random() * 3 - 1.5)).toFixed(1))));
    }, 2000);
    return () => clearInterval(timer);
  }, []);

  const getBarColor = (value: number) => {
    if (value > 85) return "bg-red-500";
    if (value > 65) return "bg-yellow-500";
    return "bg-green-500";
  };

  const getTextColor = (value: number) => {
    if (value > 85) return isDark ? "text-red-400" : "text-red-500";
    if (value > 65) return isDark ? "text-yellow-400" : "text-yellow-600";
    return isDark ? "text-green-400" : "text-green-600";
  };

  return (
    <div className={`shrink-0 flex items-center gap-6 px-5 py-1.5 border-t ${isDark ? "bg-[#1e1e1e] border-gray-700" : "bg-white border-gray-200"}`}>
      {/* CPU */}
      <div className="flex items-center gap-2">
        <Cpu style={{ width: 12, height: 12 }} className={isDark ? "text-gray-500" : "text-gray-400"} />
        <span className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 10 }}>CPU</span>
        <div className={`w-16 h-1.5 rounded-full overflow-hidden ${isDark ? "bg-gray-700" : "bg-gray-200"}`}>
          <div className={`h-full rounded-full transition-all duration-1000 ${getBarColor(cpu)}`} style={{ width: `${cpu}%` }} />
        </div>
        <span className={getTextColor(cpu)} style={{ fontSize: 10 }}>{cpu}%</span>
      </div>

      {/* RAM */}
      <div className="flex items-center gap-2">
        <MemoryStick style={{ width: 12, height: 12 }} className={isDark ? "text-gray-500" : "text-gray-400"} />
        <span className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 10 }}>RAM</span>
        <div className={`w-16 h-1.5 rounded-full overflow-hidden ${isDark ? "bg-gray-700" : "bg-gray-200"}`}>
          <div className={`h-full rounded-full transition-all duration-1000 ${getBarColor(ram)}`} style={{ width: `${ram}%` }} />
        </div>
        <span className={getTextColor(ram)} style={{ fontSize: 10 }}>{ram}%</span>
      </div>

      {/* Disk */}
      <div className="flex items-center gap-2">
        <HardDrive style={{ width: 12, height: 12 }} className={isDark ? "text-gray-500" : "text-gray-400"} />
        <span className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 10 }}>DISK</span>
        <div className={`w-16 h-1.5 rounded-full overflow-hidden ${isDark ? "bg-gray-700" : "bg-gray-200"}`}>
          <div className={`h-full rounded-full transition-all duration-1000 ${getBarColor(disk)}`} style={{ width: `${disk}%` }} />
        </div>
        <span className={getTextColor(disk)} style={{ fontSize: 10 }}>{disk}%</span>
      </div>

      {/* Separator */}
      <div className={`h-3 w-px ${isDark ? "bg-gray-700" : "bg-gray-200"}`} />

      {/* Network */}
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-1.5">
          <Upload style={{ width: 10, height: 10 }} className={isDark ? "text-gray-500" : "text-gray-400"} />
          <span className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 10 }}>{netUp} MB/s</span>
        </div>
        <div className="flex items-center gap-1.5">
          <Download style={{ width: 10, height: 10 }} className={isDark ? "text-gray-500" : "text-gray-400"} />
          <span className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 10 }}>{netDown} MB/s</span>
        </div>
      </div>

      <div className="flex-1" />

      {/* Ping */}
      {device.ping !== null && (
        <div className="flex items-center gap-1.5">
          <Activity style={{ width: 11, height: 11 }} className={device.ping < 30 ? "text-green-500" : "text-yellow-500"} />
          <span className={device.ping < 30 ? "text-green-600" : "text-yellow-600"} style={{ fontSize: 10 }}>{device.ping}ms</span>
        </div>
      )}

      {/* TLS */}
      <div className="flex items-center gap-1">
        <Lock style={{ width: 10, height: 10 }} className="text-green-500" />
        <span className="text-green-600" style={{ fontSize: 10 }}>TLS 1.3</span>
      </div>
    </div>
  );
}

/* ======================== Shared sub-components ======================== */

function ResourcePill({ label, value, color }: { label: string; value: number; color: string }) {
  const { isDark } = useTheme();
  const colorMap: Record<string, string> = {
    blue: isDark ? "text-blue-400 bg-blue-900/30" : "text-blue-600 bg-blue-50",
    purple: isDark ? "text-purple-400 bg-purple-900/30" : "text-purple-600 bg-purple-50",
    green: isDark ? "text-green-400 bg-green-900/30" : "text-green-600 bg-green-50",
  };
  const barColor: Record<string, string> = {
    blue: "bg-blue-500",
    purple: "bg-purple-500",
    green: "bg-green-500",
  };
  return (
    <div className="flex items-center gap-1.5">
      <span className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 10 }}>{label}</span>
      <div className={`w-14 h-1 rounded-full ${isDark ? "bg-gray-700" : "bg-gray-200"}`}>
        <div className={`h-full rounded-full ${barColor[color]}`} style={{ width: `${value}%`, opacity: 0.75 }} />
      </div>
      <span className={`${colorMap[color]} px-1 rounded`} style={{ fontSize: 10 }}>{value}%</span>
    </div>
  );
}

function ToolbarBtn({
  icon, label, onClick, active, danger,
}: {
  icon: React.ReactNode; label: string; onClick?: () => void; active?: boolean; danger?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      title={label}
      className={`flex items-center gap-1.5 px-2 py-1 rounded-md transition-colors ${
        danger ? "text-red-400/70 hover:bg-red-500/10 hover:text-red-400"
        : active === false ? "text-gray-500 hover:bg-white/5 hover:text-gray-300"
        : "text-gray-300 hover:bg-white/8 hover:text-gray-100"
      }`}
    >
      {icon}
      <span style={{ fontSize: 11 }}>{label}</span>
    </button>
  );
}

function StatusItem({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center gap-1.5" style={{ fontSize: 11 }}>
      <span className="text-gray-500">{label}</span>
      <span className="text-gray-300">{value}</span>
    </div>
  );
}

function StatusPanel({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-white/10 bg-black/25 px-3 py-2">
      <div className="text-[11px] uppercase tracking-wide text-gray-500">{label}</div>
      <div className="mt-1 truncate text-sm font-semibold text-gray-100">{value}</div>
    </div>
  );
}
