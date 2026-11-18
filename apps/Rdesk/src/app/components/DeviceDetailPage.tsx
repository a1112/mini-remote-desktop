import { useCallback, useState, useEffect, useRef } from "react";
import { useLocation, useParams, useNavigate } from "react-router";
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
import {
  ipcCancelFileTransfer,
  ipcListDirectory,
  ipcListFileTransferProviders,
  ipcListFileTransfers,
  type DirectoryList,
  type FileEntry,
  type FileTransferProviderDescriptor,
  type FileTransferStatus,
  type FileTransferTaskSnapshot,
} from "../adapters/tauri";
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
  Pause,
  Play,
  File,
  Folder,
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
  Laptop,
  Server,
  Smartphone,
  Info,
  ArrowRightLeft,
  ChevronUp,
  Loader2,
  AlertCircle,
} from "lucide-react";
import { useTheme } from "./ThemeContext";
import { useDetailBar } from "./DetailBarContext";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from "./ui/context-menu";

type TabType = "remote" | "files" | "apps" | "terminal" | "info";

export function deviceDetailTabFromSearch(search: string): TabType {
  const tab = new URLSearchParams(search).get("tab");
  return tab === "files" || tab === "apps" || tab === "terminal" || tab === "info" || tab === "remote"
    ? tab
    : "remote";
}

export function remoteApplicationSourceMatchesTerminalFocus(
  source: Pick<CaptureSource, "app_name" | "title">
): boolean {
  const haystack = `${source.app_name ?? ""} ${source.title ?? ""}`.toLowerCase();
  return /\b(cmd|command prompt|powershell|pwsh|terminal|wt\.exe)\b/.test(haystack);
}

export function remoteStartUnavailableReason(
  device: Pick<Device, "disabled" | "status">
): string | null {
  if (device.disabled) return "设备已禁用";
  if (device.status !== "online") return "设备当前离线";
  return null;
}

export function DeviceDetailPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const location = useLocation();
  const { devices, loading } = useDevices();
  const device = useDeviceById(id, devices);
  const [activeTab, setActiveTab] = useState<TabType>(() =>
    deviceDetailTabFromSearch(location.search)
  );
  const { isDark } = useTheme();
  const detailBar = useDetailBar();

  const Icon = device?.icon || Monitor;
  const isOnline = device?.status === "online";
  const tabs: { key: TabType; label: string; icon: typeof Monitor }[] = [
    { key: "remote", label: "远程桌面", icon: Monitor },
    { key: "files", label: "文件传输", icon: FolderOpen },
    { key: "apps", label: "远程应用", icon: AppWindow },
    { key: "terminal", label: "远程终端", icon: Terminal },
    { key: "info", label: "设备信息", icon: Info },
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

  useEffect(() => {
    setActiveTab(deviceDetailTabFromSearch(location.search));
  }, [location.search]);

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
        {activeTab === "files" && <FilesTab device={device} />}
        {activeTab === "apps" && <AppsTab device={device} />}
        {activeTab === "terminal" && <AppsTab device={device} terminalFocus />}
        {activeTab === "info" && <InfoTab device={device} />}
      </div>

      {/* Performance monitoring footer */}
      {isOnline && (device.cpu !== null || device.ram !== null || device.disk !== null) && (
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
  const remoteUnavailableReason = remoteStartUnavailableReason(device);
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
    if (remoteUnavailableReason) {
      setConnectionError(remoteUnavailableReason);
      return;
    }
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
      if (result.mode === "route") {
        navigate(result.routePath ?? `/session/${result.sessionId}`);
        return;
      }
      activeSessionIdRef.current = result.sessionId;
      setActiveSessionId(result.sessionId);
      setRemoteWindowLabel(result.windowLabel);
      setSessionSnapshot(null);
      setProbeSnapshot(null);
      setElapsed(0);
      // Only the explicit local/native test path can return a window here.
      // Secure remote requests are rendered by RemoteSessionPage after their
      // authoritative presentation state reaches `streaming`.
      setConnected(true);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Open remote display failed";
      setConnectionError(message);
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
            disabled={launching || Boolean(remoteUnavailableReason)}
            className="w-full px-8 py-2.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-white transition-colors shadow-sm disabled:cursor-not-allowed disabled:opacity-60"
            style={{ fontSize: 14 }}
          >
            {remoteUnavailableReason ?? "发起远程连接"}
          </button>
          {connectionError ? (
            <div
              role="alert"
              className={`mt-4 flex items-start gap-2 rounded-lg border px-3 py-2 text-left ${isDark ? "border-red-500/25 bg-red-500/10 text-red-200" : "border-red-200 bg-red-50 text-red-700"}`}
              style={{ fontSize: 12 }}
            >
              <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span>{connectionError}</span>
            </div>
          ) : null}
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
      <div className="flex-1 relative overflow-hidden cursor-crosshair select-none bg-[#070b14]">
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
          {device.name} · {device.os}
        </div>
      </div>

      {/* Status bar */}
      <div className="flex items-center justify-between px-4 py-1.5 bg-[#232340] border-t border-white/10 shrink-0">
        <div className="flex items-center gap-4">
          <StatusItem label="Size" value={frameSizeLabel} />
          <StatusItem label="FPS" value={fpsLabel} />
          <StatusItem label="Bitrate" value={bitrateLabel} />
        </div>
      </div>
    </div>
  );
}

/* ======================== File Transfer Tab ======================== */
function formatFileSize(sizeBytes: number | null | undefined): string {
  if (sizeBytes === null || sizeBytes === undefined) return "--";
  if (sizeBytes < 1024) return `${sizeBytes} B`;
  if (sizeBytes < 1024 * 1024) return `${Math.round(sizeBytes / 1024)} KB`;
  if (sizeBytes < 1024 * 1024 * 1024) return `${(sizeBytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(sizeBytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

function formatModifiedTime(modifiedMs: number | null | undefined): string {
  if (!modifiedMs) return "--";
  const date = new Date(modifiedMs);
  if (Number.isNaN(date.getTime())) return "--";
  return date.toISOString().slice(0, 10);
}

function fileKindLabel(entry: FileEntry): string {
  if (entry.kind === "directory") return "文件夹";
  if (entry.kind === "symlink") return "符号链接";
  if (entry.kind === "other") return "其他";
  const ext = entry.name.split(".").pop()?.toLowerCase();
  if (ext === "pdf") return "PDF 文档";
  if (ext === "json") return "JSON 文件";
  if (ext === "png" || ext === "jpg" || ext === "jpeg") return "图片";
  if (ext === "txt" || ext === "md") return "文本文件";
  return "文件";
}

function fileTransferStatusLabel(status: FileTransferStatus): string {
  switch (status) {
    case "queued":
      return "排队";
    case "running":
      return "运行中";
    case "completed":
      return "完成";
    case "failed":
      return "失败";
    case "cancelled":
      return "已取消";
    default:
      return status;
  }
}

function fileTransferProgressLabel(transfer: FileTransferTaskSnapshot): string {
  return `${fileTransferStatusLabel(transfer.status)} ${transfer.copied_entries}/${transfer.total_entries}`;
}

function fileTransferByteLabel(transfer: FileTransferTaskSnapshot): string {
  return `${formatFileSize(transfer.copied_bytes)} / ${formatFileSize(transfer.total_bytes)}`;
}

function fileTransferProviderStatusLabel(
  status: FileTransferProviderDescriptor["status"]
): string {
  switch (status) {
    case "available":
      return "可用";
    case "unimplemented":
      return "预留";
    case "unsupported":
      return "不支持";
    case "degraded":
      return "降级";
    default:
      return status;
  }
}

function isCancellableFileTransfer(status: FileTransferStatus): boolean {
  return status === "queued" || status === "running";
}

function upsertFileTransferTask(
  transfers: FileTransferTaskSnapshot[],
  nextTransfer: FileTransferTaskSnapshot
): FileTransferTaskSnapshot[] {
  const index = transfers.findIndex(
    (transfer) => transfer.transfer_id === nextTransfer.transfer_id
  );
  if (index === -1) return [nextTransfer, ...transfers];
  const nextTransfers = [...transfers];
  nextTransfers[index] = nextTransfer;
  return nextTransfers;
}

function localEntryIcon(entry: FileEntry) {
  if (entry.kind === "directory") return <Folder aria-hidden="true" className="size-4 shrink-0 text-yellow-600" />;
  if (entry.name.endsWith(".png") || entry.name.endsWith(".jpg")) return <Image aria-hidden="true" className="size-4 shrink-0 text-green-600" />;
  if (entry.name.endsWith(".pdf")) return <FileText aria-hidden="true" className="size-4 shrink-0 text-red-600" />;
  if (entry.name.endsWith(".mp3") || entry.name.endsWith(".wav")) return <Music aria-hidden="true" className="size-4 shrink-0 text-purple-600" />;
  return <File aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />;
}

function LocalDirectoryPane() {
  const [directory, setDirectory] = useState<DirectoryList | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [asTiles, setAsTiles] = useState(false);
  const [details, setDetails] = useState<FileEntry | null>(null);
  const [copyStatus, setCopyStatus] = useState<"copied" | "failed" | null>(null);
  const latestDirectoryRequest = useRef(0);

  const loadDirectory = useCallback(async (path: string | null) => {
    const request = ++latestDirectoryRequest.current;
    setLoading(true);
    setError(null);
    setDetails(null);
    setCopyStatus(null);
    try {
      const result = await ipcListDirectory(path);
      if (request !== latestDirectoryRequest.current) return;
      if (result.ok) setDirectory(result.value);
      else setError(result.error.message);
    } catch (cause) {
      if (request === latestDirectoryRequest.current) {
        setError(cause instanceof Error ? cause.message : "暂时无法读取目录");
      }
    } finally {
      if (request === latestDirectoryRequest.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadDirectory(null);
    return () => { latestDirectoryRequest.current += 1; };
  }, [loadDirectory]);

  const copyPath = async (path: string) => {
    try {
      await navigator.clipboard.writeText(path);
      setCopyStatus("copied");
    } catch {
      setCopyStatus("failed");
    }
  };

  const query = search.trim().toLowerCase();
  const entries = (directory?.entries ?? []).filter((entry) => entry.name.toLowerCase().includes(query));
  const parentPath = directory?.parent_path;
  const iconButton = "rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-blue-500 disabled:opacity-40";

  return (
    <section aria-label="本机目录" className="flex min-h-0 flex-1 flex-col overflow-hidden bg-card text-card-foreground">
      <div className="flex flex-wrap items-center gap-1.5 border-b border-border px-3 py-2">
        <button type="button" aria-label="上级目录" className={iconButton} disabled={!parentPath || loading} onClick={() => void loadDirectory(parentPath ?? null)}>
          <ArrowUp aria-hidden="true" className="size-4" />
        </button>
        <button type="button" aria-label="主目录" className={iconButton} disabled={loading} onClick={() => void loadDirectory(null)}>
          <Home aria-hidden="true" className="size-4" />
        </button>
        <button type="button" aria-label="刷新目录" className={iconButton} disabled={loading} onClick={() => void loadDirectory(directory?.path ?? null)}>
          <RefreshCw aria-hidden="true" className="size-4" />
        </button>
        <span className="min-w-24 flex-1 truncate rounded bg-muted px-2 py-1.5 font-mono text-xs" title={directory?.path}>
          {directory?.path ?? "本机目录"}
        </span>
        <label className="flex items-center gap-1.5 rounded border border-border px-2">
          <Search aria-hidden="true" className="size-3.5 text-muted-foreground" />
          <input
            type="search"
            aria-label="搜索当前目录"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="搜索当前目录"
            className="w-28 bg-transparent py-1.5 text-xs"
          />
        </label>
        <button type="button" aria-label="列表视图" aria-pressed={!asTiles} className={iconButton} onClick={() => setAsTiles(false)}>
          <List aria-hidden="true" className="size-4" />
        </button>
        <button type="button" aria-label="网格视图" aria-pressed={asTiles} className={iconButton} onClick={() => setAsTiles(true)}>
          <LayoutGrid aria-hidden="true" className="size-4" />
        </button>
      </div>

      {loading && <p role="status" className="px-3 py-2 text-xs text-muted-foreground">正在读取目录…</p>}
      {error && <p role="alert" className="break-words px-3 py-2 text-xs text-destructive">目录读取失败。{error}</p>}
      {copyStatus === "copied" && <p role="status" className="px-3 py-2 text-xs text-muted-foreground">已复制文件路径</p>}
      {copyStatus === "failed" && <p role="alert" className="px-3 py-2 text-xs text-destructive">无法复制文件路径</p>}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {!loading && !error && entries.length === 0 && (
          <p role="status" className="px-3 py-4 text-sm text-muted-foreground">
            {query ? "没有符合条件的项目" : "此目录为空"}
          </p>
        )}
        <ul className={asTiles ? "grid grid-cols-[repeat(auto-fill,minmax(8rem,1fr))] gap-2 p-3" : "divide-y divide-border"}>
          {entries.map((entry) => (
            <li key={entry.path} className="min-w-0">
              <ContextMenu>
                <ContextMenuTrigger asChild>
                  <button
                    type="button"
                    onClick={() => entry.kind === "directory" ? void loadDirectory(entry.path) : setDetails(entry)}
                    className={asTiles
                      ? "flex w-full flex-col items-center gap-2 rounded border border-border p-3 text-xs hover:bg-muted focus-visible:ring-2 focus-visible:ring-blue-500"
                      : "flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-muted focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-blue-500"}
                  >
                    {localEntryIcon(entry)}
                    <span className="max-w-full min-w-0 flex-1 truncate">{entry.name}</span>
                    {!asTiles && <>
                      <span className="w-24 shrink-0 text-right text-muted-foreground">{formatModifiedTime(entry.modified_ms)}</span>
                      <span className="w-20 shrink-0 text-right text-muted-foreground">{fileKindLabel(entry)}</span>
                      <span className="w-16 shrink-0 text-right text-muted-foreground">{formatFileSize(entry.size_bytes)}</span>
                    </>}
                  </button>
                </ContextMenuTrigger>
                <ContextMenuContent>
                  {entry.kind === "directory" && <ContextMenuItem onSelect={() => void loadDirectory(entry.path)}>
                    <FolderOpen aria-hidden="true" />打开
                  </ContextMenuItem>}
                  <ContextMenuItem onSelect={() => void copyPath(entry.path)}>
                    <Copy aria-hidden="true" />复制路径
                  </ContextMenuItem>
                  <ContextMenuItem onSelect={() => setDetails(entry)}>
                    <Info aria-hidden="true" />属性
                  </ContextMenuItem>
                </ContextMenuContent>
              </ContextMenu>
            </li>
          ))}
        </ul>
      </div>

      {details && <section className="shrink-0 border-t border-border bg-muted/50 px-3 py-2" aria-label="文件属性">
        <div className="mb-2 flex items-center justify-between gap-2">
          <h3 className="min-w-0 truncate text-sm font-medium">{details.name}</h3>
          <button type="button" className={iconButton} onClick={() => setDetails(null)}>关闭属性</button>
        </div>
        <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
          <dt>路径</dt><dd className="break-all font-mono">{details.path}</dd>
          <dt>类型</dt><dd>{fileKindLabel(details)}</dd>
          <dt>修改日期</dt><dd>{formatModifiedTime(details.modified_ms)}</dd>
          <dt>大小</dt><dd>{formatFileSize(details.size_bytes)}</dd>
          <dt>只读</dt><dd>{details.readonly ? "是" : "否"}</dd>
        </dl>
      </section>}
    </section>
  );
}

function FilesTab({ device }: { device: Device }) {
  const { isDark } = useTheme();
  const native = isTauriRuntime();
  const unavailableReason = remoteStartUnavailableReason(device);

  const [transferError, setTransferError] = useState<string | null>(null);
  const [fileTransfers, setFileTransfers] = useState<FileTransferTaskSnapshot[]>([]);
  const [transferListError, setTransferListError] = useState<string | null>(null);
  const [fileTransferProviders, setFileTransferProviders] = useState<FileTransferProviderDescriptor[]>([]);
  const [providerListError, setProviderListError] = useState<string | null>(null);
  const [cancellingTransferId, setCancellingTransferId] = useState<string | null>(null);

  const refreshFileTransfers = useCallback(async () => {
    const result = await ipcListFileTransfers();
    if (!result.ok) {
      setTransferListError(result.error.message);
      return;
    }
    setTransferListError(null);
    setFileTransfers(result.value);
  }, []);

  const refreshFileTransferProviders = useCallback(async () => {
    const result = await ipcListFileTransferProviders();
    if (!result.ok) {
      setProviderListError(result.error.message);
      return;
    }
    setProviderListError(null);
    setFileTransferProviders(result.value);
  }, []);

  useEffect(() => {
    if (!native || unavailableReason) return;
    void refreshFileTransfers();
    void refreshFileTransferProviders();
  }, [native, refreshFileTransferProviders, refreshFileTransfers, unavailableReason]);

  const handleCancelFileTransfer = useCallback(async (transferId: string) => {
    setCancellingTransferId(transferId);
    setTransferError(null);
    const result = await ipcCancelFileTransfer(transferId);
    setCancellingTransferId(null);
    if (!result.ok) {
      setTransferError(result.error.message);
      return;
    }
    setFileTransfers((transfers) => upsertFileTransferTask(transfers, result.value));
  }, []);

  if (!native) {
    return <p className="p-6 text-sm text-muted-foreground">网页端暂不支持文件操作。可在桌面客户端浏览本机目录。</p>;
  }

  if (unavailableReason) {
    return (
      <div className={`flex items-center justify-center h-full ${isDark ? "bg-[#1a1a1a]" : "bg-[#f0f2f5]"}`}>
        <div className="text-center">
          <WifiOff className={`w-12 h-12 mx-auto mb-3 ${isDark ? "text-gray-600" : "text-gray-300"}`} />
          <div className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 16 }}>{unavailableReason}，无法传输文件</div>
        </div>
      </div>
    );
  }

  return (
    <div className={`relative flex h-full flex-col overflow-hidden ${isDark ? "bg-[#1a1a1a]" : "bg-[#f0f2f5]"}`}>
      {transferError && <p role="alert" className="px-4 py-2 text-xs text-destructive">{transferError}</p>}
      {(fileTransfers.length > 0 || transferListError) && (
        <div
          role="region"
          aria-label="传输任务"
          className={`absolute bottom-3 right-4 z-30 w-[380px] max-w-[calc(100%-2rem)] rounded-lg border shadow-lg ${
            isDark
              ? "border-gray-700 bg-[#202020] text-gray-200"
              : "border-gray-200 bg-white text-gray-800"
          }`}
        >
          <div className={`flex items-center justify-between border-b px-3 py-2 ${isDark ? "border-gray-700" : "border-gray-100"}`}>
            <div className="flex items-center gap-2">
              <ArrowRightLeft className="h-3.5 w-3.5 text-blue-500" />
              <span className="font-medium" style={{ fontSize: 12 }}>传输任务</span>
            </div>
            <button
              onClick={() => void refreshFileTransfers()}
              className={`rounded-md p-1 transition-colors ${isDark ? "text-gray-400 hover:bg-gray-700 hover:text-gray-200" : "text-gray-500 hover:bg-gray-100 hover:text-gray-700"}`}
              title="刷新传输任务"
            >
              <RefreshCw className="h-3.5 w-3.5" />
            </button>
          </div>
          {transferListError ? (
            <div className={`px-3 py-2 ${isDark ? "text-red-300" : "text-red-700"}`} style={{ fontSize: 11 }}>
              读取传输任务失败：{transferListError}
            </div>
          ) : (
            <div className="max-h-48 overflow-y-auto py-1">
              {fileTransfers.map((transfer) => (
                <div
                  key={transfer.transfer_id}
                  className={`flex items-start gap-2 px-3 py-2 ${isDark ? "hover:bg-[#292929]" : "hover:bg-gray-50"}`}
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate font-mono" style={{ fontSize: 11 }}>{transfer.transfer_id}</span>
                      <span
                        className={`shrink-0 rounded px-1.5 py-0.5 ${
                          transfer.status === "failed"
                            ? isDark ? "bg-red-950 text-red-300" : "bg-red-50 text-red-700"
                            : transfer.status === "completed"
                              ? isDark ? "bg-emerald-950 text-emerald-300" : "bg-emerald-50 text-emerald-700"
                              : isDark ? "bg-blue-950 text-blue-300" : "bg-blue-50 text-blue-700"
                        }`}
                        style={{ fontSize: 10 }}
                      >
                        {fileTransferProgressLabel(transfer)}
                      </span>
                    </div>
                    <div className={`mt-1 flex items-center gap-2 ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 10 }}>
                      <span>{fileTransferByteLabel(transfer)}</span>
                      <span>·</span>
                      <span>{transfer.transport_kind}</span>
                      <span>·</span>
                      <span>{transfer.provider_kind ?? "mrd-local"}</span>
                    </div>
                    {transfer.error ? (
                      <div className={isDark ? "mt-1 text-red-300" : "mt-1 text-red-700"} style={{ fontSize: 10 }}>
                        {transfer.error}
                      </div>
                    ) : null}
                  </div>
                  {isCancellableFileTransfer(transfer.status) ? (
                    <button
                      onClick={() => void handleCancelFileTransfer(transfer.transfer_id)}
                      disabled={cancellingTransferId === transfer.transfer_id}
                      aria-label={`取消 ${transfer.transfer_id}`}
                      className={`shrink-0 rounded-md border px-2 py-1 transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${
                        isDark
                          ? "border-gray-700 text-gray-300 hover:border-red-500 hover:text-red-300"
                          : "border-gray-200 text-gray-600 hover:border-red-300 hover:text-red-600"
                      }`}
                      style={{ fontSize: 11 }}
                    >
                      {cancellingTransferId === transfer.transfer_id ? "取消中" : "取消"}
                    </button>
                  ) : null}
                </div>
              ))}
            </div>
          )}
        </div>
      )}
      {(fileTransferProviders.length > 0 || providerListError) && (
        <div
          role="region"
          aria-label="传输 Provider"
          className={`shrink-0 border-b px-4 py-2 ${
            isDark
              ? "border-gray-700 bg-[#202020] text-gray-200"
              : "border-gray-200 bg-white text-gray-800"
          }`}
        >
          <div className="flex flex-wrap items-center gap-2">
            <div className="flex items-center gap-2">
              <Server className="h-3.5 w-3.5 text-indigo-500" />
              <span className="font-medium" style={{ fontSize: 12 }}>传输 Provider</span>
            </div>
            {providerListError ? (
              <span className={isDark ? "text-red-300" : "text-red-700"} style={{ fontSize: 11 }}>
                读取失败：{providerListError}
              </span>
            ) : (
              fileTransferProviders.map((provider) => (
                <div key={provider.provider_kind} className={`flex min-w-0 max-w-full flex-col gap-1 rounded-md border px-2 py-1 ${
                  isDark ? "border-gray-700 bg-[#252525]" : "border-gray-200 bg-gray-50"
                }`}>
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="truncate" style={{ fontSize: 11 }}>
                      {provider.display_name}
                    </span>
                    <span className={isDark ? "font-mono text-gray-500" : "font-mono text-gray-400"} style={{ fontSize: 10 }}>
                      {provider.provider_kind}
                    </span>
                    <span
                      title={provider.reason ?? provider.capabilities?.join(", ") ?? provider.status}
                      className={`shrink-0 rounded px-1.5 py-0.5 ${
                        provider.status === "available"
                          ? isDark ? "bg-emerald-950 text-emerald-300" : "bg-emerald-50 text-emerald-700"
                          : isDark ? "bg-amber-950 text-amber-300" : "bg-amber-50 text-amber-700"
                      }`}
                      style={{ fontSize: 10 }}
                    >
                      {fileTransferProviderStatusLabel(provider.status)}
                    </span>
                  </div>
                  {provider.handoff_hint ? (
                    <div className={`flex min-w-0 flex-wrap items-center gap-1 ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 10 }}>
                      <span>{provider.handoff_hint.external_app}</span>
                      <span>·</span>
                      <span className="font-mono">{provider.handoff_hint.bridge_service}</span>
                      {provider.handoff_hint.control_endpoint ? (
                        <>
                          <span>·</span>
                          <span className="font-mono">{provider.handoff_hint.control_endpoint}</span>
                        </>
                      ) : null}
                    </div>
                  ) : null}
                </div>
              ))
            )}
            <button
              onClick={() => void refreshFileTransferProviders()}
              className={`rounded-md p-1 transition-colors ${isDark ? "text-gray-400 hover:bg-gray-700 hover:text-gray-200" : "text-gray-500 hover:bg-gray-100 hover:text-gray-700"}`}
              title="刷新传输 Provider"
            >
              <RefreshCw className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      )}
      {device.isLocal ? <LocalDirectoryPane key={device.id} /> : (
        <section aria-label="远程文件" className="flex flex-1 flex-col items-center justify-center gap-2 px-6 py-8 text-center">
          <FolderOpen aria-hidden="true" className="size-9 text-muted-foreground" />
          <h3 className="font-medium">远程文件不可用</h3>
          <p className="max-w-sm text-sm text-muted-foreground">远程目录浏览尚未接入服务，当前无法浏览或传输此设备的文件。</p>
        </section>
      )}
    </div>
  );
}

/* ======================== Remote Apps Tab ======================== */
function AppsTab({
  device,
  terminalFocus: terminalFocusProp = false,
}: {
  device: Device;
  terminalFocus?: boolean;
}) {
  const { isDark } = useTheme();
  const navigate = useNavigate();
  const location = useLocation();
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
  const remoteUnavailableReason = remoteStartUnavailableReason(device);
  const desktopRuntime = isTauriRuntime();
  const isLanP2PRemote = device.p2pAvailable && !device.isLocal;
  const canUseRemoteApplications = !remoteUnavailableReason && desktopRuntime && isLanP2PRemote;

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
    if (remoteUnavailableReason) {
      setAppsError(remoteUnavailableReason);
      return;
    }
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
      if (result.mode === "route") navigate(result.routePath ?? `/session/${result.sessionId}`);
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
      if (result.mode === "route") navigate(result.routePath ?? `/session/${result.sessionId}`);
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

  const unavailableReason =
    remoteUnavailableReason ??
    (!desktopRuntime
      ? "远程应用需要桌面端运行"
      : device.isLocal
        ? "本机设备请使用本地测试工作台"
        : !device.p2pAvailable
          ? "当前设备未建立 LAN P2P 通道"
          : null);
  const remoteWindows = catalog?.windows ?? [];
  const displaySources = catalog?.displays ?? [];
  const terminalFocus =
    terminalFocusProp || new URLSearchParams(location.search).get("tab") === "terminal";
  const visibleRemoteWindows = terminalFocus
    ? remoteWindows.filter(remoteApplicationSourceMatchesTerminalFocus)
    : remoteWindows;
  const orderedRemoteWindows = terminalFocus
    ? [...visibleRemoteWindows].sort((left, right) =>
        Number(remoteApplicationSourceMatchesTerminalFocus(right)) -
        Number(remoteApplicationSourceMatchesTerminalFocus(left))
      )
    : visibleRemoteWindows;

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
              {terminalFocus ? (
                <Terminal className="h-5 w-5 text-cyan-500" />
              ) : (
                <AppWindow className="h-5 w-5 text-cyan-500" />
              )}
            </div>
            <div className="min-w-0 flex-1">
              <div className={`font-semibold ${isDark ? "text-gray-100" : "text-gray-900"}`} style={{ fontSize: 16 }}>{terminalFocus ? "远程终端" : "远程应用"}</div>
              <div className={`mt-0.5 truncate ${isDark ? "text-gray-500" : "text-gray-500"}`} style={{ fontSize: 12 }}>
                {device.name} · {device.ip} · LAN QUIC 窗口流
              </div>
            </div>
            <div className={`rounded-lg border px-3 py-1.5 ${isDark ? "border-gray-700 bg-[#181818] text-gray-400" : "border-gray-200 bg-gray-50 text-gray-600"}`} style={{ fontSize: 12 }}>
              {catalog ? `${visibleRemoteWindows.length} 个窗口 / ${displaySources.length} 个屏幕` : "等待枚举"}
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

        {!sourcesLoading && catalog && visibleRemoteWindows.length === 0 && (
          <div className={`rounded-xl border p-6 text-center ${isDark ? "border-gray-700 bg-[#202020]" : "border-gray-200 bg-white"}`}>
            {terminalFocus ? (
              <Terminal className={`mx-auto mb-3 h-10 w-10 ${isDark ? "text-gray-600" : "text-gray-300"}`} />
            ) : (
              <AppWindow className={`mx-auto mb-3 h-10 w-10 ${isDark ? "text-gray-600" : "text-gray-300"}`} />
            )}
            <div className={isDark ? "text-gray-300" : "text-gray-700"} style={{ fontSize: 15 }}>
              {terminalFocus ? "未发现远程终端窗口" : "未发现可独立捕获的窗口"}
            </div>
            <div className={`mt-1 ${isDark ? "text-gray-500" : "text-gray-500"}`} style={{ fontSize: 12 }}>
              {terminalFocus
                ? `已发现 ${remoteWindows.length} 个窗口，但没有匹配 PowerShell、cmd 或 Windows Terminal。`
                : `已发现 ${catalog.sources.length} 个采集源，可先打开远程桌面或在远端启动目标应用后刷新。`}
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
            {orderedRemoteWindows.map((source) => {
              const SourceIcon = remoteCaptureSourceIcon(source);
              const opening = openingSourceId === source.id;
              return (
                <div
                  key={source.id}
                  className={`group overflow-hidden rounded-xl border shadow-sm transition-colors ${isDark ? "border-gray-700 bg-[#202020] hover:border-cyan-700" : "border-gray-200 bg-white hover:border-cyan-300"}`}
                >
                  <div className={`flex h-28 items-center justify-center border-b ${isDark ? "border-gray-700 bg-[#151515]" : "border-gray-100 bg-gray-50"}`}>
                    <div className={`flex h-14 w-14 items-center justify-center rounded-2xl ${remoteCaptureSourceAccent(source)}`}>
                      <SourceIcon className="h-7 w-7 text-white" />
                    </div>
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

/* ======================== Device Info Tab ======================== */
function InfoTab({ device }: { device: Device }) {
  const { isDark } = useTheme();
  const Icon = device.icon;
  const statusLabel = device.status === "online" ? "在线" : "离线";
  const discoverySourceLabel = device.discoverySources
    .map((source) => {
      if (source === "lan_p2p") return "P2P 局域网";
      if (source === "server") return "服务器";
      if (source === "local") return "本机";
      return source;
    })
    .join(" / ");

  const rows = [
    ["设备 ID", device.deviceId],
    ["系统", device.os],
    ["地址", device.ip],
    ["位置", device.location],
    ["分组", device.group],
    ["最后在线", device.lastSeen],
    ["发现来源", device.sourceLabel],
    ["原始来源", discoverySourceLabel || "未知"],
  ];

  const capabilityBadges = [
    device.p2pAvailable ? "P2P 可用" : "P2P 不可用",
    device.serverAvailable ? "服务器可用" : "服务器不可用",
    device.isLocal ? "本机设备" : "远程设备",
    device.favorite ? "已收藏" : "未收藏",
  ];

  return (
    <div className={`h-full overflow-y-auto ${isDark ? "bg-[#181818]" : "bg-[#f6f7f9]"}`}>
      <div className="mx-auto flex max-w-5xl flex-col gap-4 px-6 py-5">
        <div className={`flex items-center gap-4 border-b pb-4 ${isDark ? "border-gray-700" : "border-gray-200"}`}>
          <div className={`flex h-12 w-12 shrink-0 items-center justify-center rounded-lg ${device.status === "online" ? (isDark ? "bg-blue-900/30" : "bg-blue-50") : (isDark ? "bg-gray-800" : "bg-gray-100")}`}>
            <Icon className={device.status === "online" ? "text-blue-600" : "text-gray-400"} style={{ width: 24, height: 24 }} />
          </div>
          <div className="min-w-0 flex-1">
            <div className={`truncate font-medium ${isDark ? "text-gray-100" : "text-gray-900"}`} style={{ fontSize: 18 }}>{device.name}</div>
            <div className={`mt-1 flex flex-wrap items-center gap-2 ${isDark ? "text-gray-400" : "text-gray-500"}`} style={{ fontSize: 12 }}>
              <span>{statusLabel}</span>
              <span>{device.os}</span>
              <span>{device.ip}</span>
              {device.ping !== null && <span>{device.ping}ms</span>}
            </div>
          </div>
        </div>

        <div className="grid gap-4 lg:grid-cols-[1fr_280px]">
          <div className={`overflow-hidden rounded-lg border ${isDark ? "border-gray-700 bg-[#202020]" : "border-gray-200 bg-white"}`}>
            {rows.map(([label, value], index) => (
              <div
                key={label}
                className={`grid grid-cols-[120px_1fr] gap-4 px-4 py-3 ${
                  index > 0 ? isDark ? "border-t border-gray-700" : "border-t border-gray-100" : ""
                }`}
              >
                <div className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 12 }}>{label}</div>
                <div className={`min-w-0 break-all font-medium ${isDark ? "text-gray-200" : "text-gray-800"}`} style={{ fontSize: 13 }}>{value}</div>
              </div>
            ))}
          </div>

          <div className="flex flex-col gap-4">
            <div className={`rounded-lg border p-4 ${isDark ? "border-gray-700 bg-[#202020]" : "border-gray-200 bg-white"}`}>
              <div className={`mb-3 font-medium ${isDark ? "text-gray-200" : "text-gray-800"}`} style={{ fontSize: 13 }}>连接状态</div>
              <div className="flex flex-wrap gap-2">
                {capabilityBadges.map((badge) => (
                  <span
                    key={badge}
                    className={`rounded px-2 py-1 ${isDark ? "bg-gray-800 text-gray-300" : "bg-gray-100 text-gray-700"}`}
                    style={{ fontSize: 12 }}
                  >
                    {badge}
                  </span>
                ))}
              </div>
            </div>

            {(device.cpu !== null || device.ram !== null || device.disk !== null) && (
              <div className={`rounded-lg border p-4 ${isDark ? "border-gray-700 bg-[#202020]" : "border-gray-200 bg-white"}`}>
                <div className={`mb-3 font-medium ${isDark ? "text-gray-200" : "text-gray-800"}`} style={{ fontSize: 13 }}>资源</div>
                <div className="space-y-2">
                  {device.cpu !== null && <ResourcePill label="CPU" value={device.cpu} color="blue" />}
                  {device.ram !== null && <ResourcePill label="RAM" value={device.ram} color="purple" />}
                  {device.disk !== null && <ResourcePill label="DISK" value={device.disk} color="green" />}
                </div>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

/* ======================== Performance Monitoring Footer ======================== */
function PerformanceFooter({ device }: { device: Device }) {
  const { isDark } = useTheme();
  const metrics = [
    { label: "CPU", value: device.cpu },
    { label: "RAM", value: device.ram },
    { label: "DISK", value: device.disk },
  ].filter((metric) => metric.value !== null);
  return <div className={isDark ? "shrink-0 flex gap-5 px-5 py-2 border-t border-gray-700 bg-[#1e1e1e]" : "shrink-0 flex gap-5 px-5 py-2 border-t border-gray-200 bg-white"}>
    {metrics.map((metric) => <span key={metric.label} className="text-xs opacity-70">{metric.label} {metric.value}%</span>)}
  </div>;
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
