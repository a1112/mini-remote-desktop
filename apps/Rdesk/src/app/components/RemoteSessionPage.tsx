import { useState, useEffect } from "react";
import { useParams, useNavigate } from "react-router";
import { type Device, useDeviceById, useDevices } from "./deviceData";
import {
  X,
  Minus,
  Square,
  Monitor,
  Keyboard,
  Mouse,
  Volume2,
  VolumeX,
  Clipboard,
  Wifi,
  Power,
  RefreshCw,
  Lock,
  Minimize2,
  ArrowLeft,
  Signal,
} from "lucide-react";
import { useTheme } from "./ThemeContext";
import {
  getDecodePolicy,
  getNvdecRuntimeProbe,
  setDecodePolicy,
  type DecodePolicyResponse,
  type DecoderPolicy,
  type NvdecRuntimeProbe,
} from "../services/realtimeService";
import {
  getWebrtcHostSnapshot,
  type WebrtcHostSnapshot,
} from "../services/realtimeSessionService";
import { openRemoteDisplayWindow } from "../adapters/tauri";
import { withTauriWindow } from "../utils/tauriWindow";
import { isTauriRuntime } from "../utils/runtime";
import {
  getWebRemoteSession,
  type WebRemoteSession,
} from "../services/webRemoteSessionService";
// DEPRECATED: Rendering services removed - now managed by mrd-service
// import {
//   attachRenderHostSession,
//   bindRenderSurfaceSource,
//   detachRenderHostSession,
//   getRenderHostSnapshot,
//   type RenderHostSnapshot,
// } from "../services/renderHostService";
// import {
//   bindCurrentRenderWindowSurface,
//   closeRenderWindow,
//   createRenderSurface,
//   getCurrentRenderSurface,
//   getRenderWindowContext,
//   listRenderSurfaces,
//   listRenderWindows,
//   openRenderWindow,
//   openRenderSurfaceWindow,
//   selectCurrentRenderSurface,
//   type RenderSurfaceDescriptor,
//   type RenderWindowContext,
// } from "../services/renderWindowService";

// Placeholder types for disabled rendering features.
type RenderFrame = {
  width: number;
  height: number;
  bytes: number;
};

type RenderHostSnapshot = {
  preview_data_url?: string;
  frame?: RenderFrame;
  renderer_backend?: string;
  surface_count?: number;
  renderer_snapshot?: {
    uploaded_frame_count?: number;
  };
  available_source_ids: string[];
  surface_source_bindings: Array<{
    surface_id: string;
    source_id: string;
  }>;
};

type RenderWindowContext = {
  label: string;
  surface_id: string;
  role: string;
  renderer_attached: boolean;
};

type RenderSurfaceDescriptor = {
  surface_id: string;
  name: string;
  role: string;
  current?: boolean;
};

function webSessionToDevice(session: WebRemoteSession): Device {
  return {
    id: session.sessionId,
    name: session.targetDeviceName,
    deviceId: session.targetDeviceId,
    os: session.targetOs,
    icon: Monitor,
    status: "online",
    location: session.mode === "web_to_local" ? "Local browser" : "Web remote",
    ping: 1,
    lastSeen: "just now",
    cpu: null,
    ram: null,
    disk: null,
    ip: session.targetIp,
    group: "WebRTC",
    favorite: false,
  };
}

export function RemoteSessionPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const { isDark } = useTheme();
  const { devices, loading } = useDevices();
  const routeDevice = useDeviceById(id, devices);
  const webRemoteSession = id ? getWebRemoteSession(id) : null;
  const device = routeDevice ?? (webRemoteSession ? webSessionToDevice(webRemoteSession) : undefined);
  const isWebRemoteSession = Boolean(webRemoteSession && !routeDevice);

  const [muted, setMuted] = useState(false);
  const [latency, setLatency] = useState(device?.ping ?? 24);
  const [quality, setQuality] = useState(87);
  const [elapsed, setElapsed] = useState(0);
  const [isMaximized, setIsMaximized] = useState(false);
  // Rendering features disabled - now managed by mrd-service
  const [renderFeaturesDisabled, setRenderFeaturesDisabled] = useState(true);
  const [nvdecProbe, setNvdecProbe] = useState<NvdecRuntimeProbe | null>(null);
  const [decodePolicy, setDecodePolicyState] = useState<DecodePolicyResponse | null>(null);
  const [webrtcHostSnapshot, setWebrtcHostSnapshot] = useState<WebrtcHostSnapshot | null>(null);
  const [webRtcState, setWebRtcState] = useState<"idle" | "connecting" | "connected" | "failed">("idle");
  const [webRtcMessage, setWebRtcMessage] = useState("WebRTC waiting");

  const noDragSelector =
    'button, a, input, select, textarea, [role="button"], [role="menuitem"], [role="menuitemcheckbox"], [role="menuitemradio"], [data-radix-collection-item], [data-no-drag="true"]';

  useEffect(() => {
    const timer = setInterval(() => {
      setElapsed((e) => e + 1);
      setLatency((l) => Math.max(10, Math.min(60, l + Math.floor(Math.random() * 7) - 3)));
      setQuality((q) => Math.max(70, Math.min(98, q + Math.floor(Math.random() * 5) - 2)));
    }, 1000);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!webRemoteSession || isTauriRuntime()) return;
    if (typeof RTCPeerConnection === "undefined") {
      setWebRtcState("failed");
      setWebRtcMessage("WebRTC is not available in this browser");
      return;
    }

    let cancelled = false;
    const left = new RTCPeerConnection();
    const right = new RTCPeerConnection();
    const channel = left.createDataChannel("rdesk-control");

    const setSafeState = (state: typeof webRtcState, message: string) => {
      if (cancelled) return;
      setWebRtcState(state);
      setWebRtcMessage(message);
    };

    setSafeState("connecting", "Creating browser WebRTC control channel");

    left.onicecandidate = (event) => {
      if (event.candidate) void right.addIceCandidate(event.candidate);
    };
    right.onicecandidate = (event) => {
      if (event.candidate) void left.addIceCandidate(event.candidate);
    };
    left.onconnectionstatechange = () => {
      if (left.connectionState === "connected") {
        setSafeState("connected", "WebRTC control channel connected");
      } else if (left.connectionState === "failed" || left.connectionState === "closed") {
        setSafeState("failed", `WebRTC ${left.connectionState}`);
      }
    };
    right.ondatachannel = (event) => {
      event.channel.onmessage = (message) => {
        setSafeState("connected", `WebRTC message: ${String(message.data)}`);
      };
    };
    channel.onopen = () => {
      channel.send(`web-remote-ready:${webRemoteSession.sessionId}`);
      setSafeState("connected", "WebRTC control channel connected");
    };
    channel.onerror = () => {
      setSafeState("failed", "WebRTC data channel error");
    };

    void (async () => {
      try {
        const offer = await left.createOffer();
        await left.setLocalDescription(offer);
        await right.setRemoteDescription(offer);
        const answer = await right.createAnswer();
        await right.setLocalDescription(answer);
        await left.setRemoteDescription(answer);
      } catch (error) {
        setSafeState(
          "failed",
          error instanceof Error ? error.message : "WebRTC setup failed"
        );
      }
    })();

    return () => {
      cancelled = true;
      channel.close();
      left.close();
      right.close();
    };
  }, [webRemoteSession?.sessionId]);

  useEffect(() => {
    void withTauriWindow(async (appWindow) => {
      const next = await appWindow.isMaximized();
      if (typeof next === "boolean") setIsMaximized(next);
    });
  }, []);

  // Render host attachment disabled - now managed by mrd-service internally
  // useEffect(() => {
  //   if (!id || !isTauriRuntime()) return;
  //   ... rendering setup code ...
  // }, [id]);

  // WebRTC host snapshot disabled - using deprecated service
  // useEffect(() => {
  //   ... webrtc snapshot code ...
  // }, [id]);

  // Render surfaces disabled - now managed by mrd-service
  // useEffect(() => {
  //   ... surfaces setup code ...
  // }, [id]);

  // Render window context refresh disabled - functions no longer available
  // useEffect(() => {
  //   if (!isTauriRuntime()) return;
  //   ... context refresh code ...
  // }, []);

  // NVDEC probe and decode policy refresh disabled - deprecated functions
  // useEffect(() => {
  //   ... nvdec probe refresh code ...
  // }, []);

  const handleUpdateDecodePolicy = async (_nextPolicy: DecoderPolicy) => {
    alert("Decode Policy 功能已迁移到 mrd-service。此功能暂时不可用。");
  };

  // Render windows refresh disabled - functions no longer available
  // useEffect(() => {
  //   if (!id || !isTauriRuntime()) return;
  //   ... windows refresh code ...
  // }, [id]);

  const formatTime = (s: number) => {
    const m = Math.floor(s / 60);
    const sec = s % 60;
    return `${m.toString().padStart(2, "0")}:${sec.toString().padStart(2, "0")}`;
  };

  const handleDisconnect = () => {
    if (isWebRemoteSession) {
      navigate("/devices");
      return;
    }
    if (device) navigate(`/devices/${device.id}`);
    else navigate("/");
  };

  // Rendering functions disabled - features now managed by mrd-service
  const handlePopOutWindow = async () => {
    if (!id) return;
    if (!isTauriRuntime()) {
      navigate(`/session/${id}`);
      return;
    }
    const result = await openRemoteDisplayWindow({ sessionId: id });
    if (!result.ok) {
      alert(result.error.message);
    }
  };

  const handleOpenCurrentSurfaceWindow = async () => {
    alert("Surface 窗口功能已迁移到 mrd-service。此功能暂时不可用。");
  };

  const handleCloseRenderWindow = async (_label: string) => {
    alert("渲染窗口功能已迁移到 mrd-service。此功能暂时不可用。");
  };

  const handleCreateSurface = async () => {
    alert("Surface 创建功能已迁移到 mrd-service。此功能暂时不可用。");
  };

  const handleSelectSurface = async (_surfaceId: string) => {
    // No-op for now
  };

  const handleOpenSelectedSurfaceWindow = async () => {
    alert("Surface 窗口功能已迁移到 mrd-service。此功能暂时不可用。");
  };

  const handleBindCurrentWindowSurface = async () => {
    alert("Surface 绑定功能已迁移到 mrd-service。此功能暂时不可用。");
  };

  const handleBindSurfaceSource = async () => {
    alert("Source 绑定功能已迁移到 mrd-service。此功能暂时不可用。");
  };

  // Placeholder values for disabled rendering features
  const [newSurfaceName, setNewSurfaceName] = useState("");
  const [selectedSourceId, setSelectedSourceId] = useState<string | null>(null);
  const [selectedSessionSurfaceId, setSelectedSessionSurfaceId] = useState<string | null>(null);
  const renderWindows: RenderWindowContext[] = [];
  const currentSurfaceId: string | null = null;
  const rendererAttached = false;
  const currentRenderWindowCount = 0;
  const renderSurfaces: RenderSurfaceDescriptor[] = [];
  const renderSnapshot = null as RenderHostSnapshot | null;
  const currentRenderWindowLabel: string | null = null;
  const currentWindowRole: string | null = null;

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

  const handleCloseWindow = () => {
    void withTauriWindow((appWindow) => appWindow.close());
  };

  if (loading && !webRemoteSession) {
    return <div className="flex items-center justify-center h-screen bg-[#1a1a1a] text-gray-400">加载设备中...</div>;
  }

  if (!device) {
    return (
      <div className="flex items-center justify-center h-screen bg-[#1a1a1a]">
        <div className="text-center">
          <div className="text-gray-500 mb-2" style={{ fontSize: 48 }}>?</div>
          <div className="text-gray-400" style={{ fontSize: 16 }}>设备未找到</div>
          <button
            onClick={() => navigate("/")}
            className="mt-3 px-4 py-2 rounded-lg bg-blue-600 text-white hover:bg-blue-500 transition-colors"
            style={{ fontSize: 13 }}
          >
            返回首页
          </button>
        </div>
      </div>
    );
  }

  const Icon = device.icon;

  return (
    <div className="flex flex-col h-screen w-screen bg-[#1a1a2e] overflow-hidden">
      {/* Title bar — unified window style with all controls */}
      <div
        className="flex items-center h-11 bg-[#232340] border-b border-white/10 shrink-0 select-none"
        style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
        onMouseDown={handleTauriDragStart}
        onDoubleClick={handleDragDoubleClick}
      >
        {/* Left: back + device info */}
        <div className="flex items-center gap-2 px-3 shrink-0" style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}>
          <button
            onClick={handleDisconnect}
            className="p-1 rounded-md text-gray-400 hover:text-gray-200 hover:bg-white/8 transition-colors"
            title="返回"
          >
            <ArrowLeft style={{ width: 14, height: 14 }} />
          </button>

          <div className="w-px h-4 bg-white/10" />

          <div className="flex items-center gap-2">
            <div className="relative w-6 h-6 rounded-md bg-blue-900/40 flex items-center justify-center">
              <Icon style={{ width: 13, height: 13 }} className="text-blue-400" />
              <div className="absolute -bottom-0.5 -right-0.5 w-2 h-2 rounded-full bg-green-500 border border-[#232340]" />
            </div>
            <div className="flex items-center gap-1.5">
              <span className="text-gray-200" style={{ fontSize: 13 }}>{device.name}</span>
              <span className="text-gray-500" style={{ fontSize: 11 }}>{device.os}</span>
            </div>
          </div>
        </div>

        {/* Center: session controls */}
        <div className="flex-1 flex items-center justify-center gap-1" style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}>
          <CtrlBtn icon={<Mouse style={{ width: 13, height: 13 }} />} label="鼠标" />
          <CtrlBtn icon={<Keyboard style={{ width: 13, height: 13 }} />} label="键盘" />
          <CtrlBtn
            icon={muted ? <VolumeX style={{ width: 13, height: 13 }} /> : <Volume2 style={{ width: 13, height: 13 }} />}
            label={muted ? "静音" : "音频"}
            onClick={() => setMuted(!muted)}
            active={!muted}
          />
          <CtrlBtn icon={<Clipboard style={{ width: 13, height: 13 }} />} label="剪贴板" />

          <div className="w-px h-4 bg-white/10 mx-0.5" />

          <CtrlBtn icon={<Lock style={{ width: 13, height: 13 }} />} label="锁屏" />
          <CtrlBtn icon={<RefreshCw style={{ width: 12, height: 12 }} />} label="刷新" />
          <CtrlBtn icon={<Power style={{ width: 13, height: 13 }} />} label="关机" danger />
        </div>

        {/* Right: status + window controls */}
        <div className="flex items-center gap-1.5 pr-1 shrink-0" style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}>
          {/* Status indicators */}
          <div className="flex items-center gap-1 px-2 py-0.5 rounded-md bg-white/6 mr-1">
            <Wifi style={{ width: 11, height: 11 }} className={latency < 30 ? "text-green-400" : latency < 50 ? "text-yellow-400" : "text-red-400"} />
            <span className="text-gray-300" style={{ fontSize: 11 }}>{latency}ms</span>
            <div className="w-px h-3 bg-white/10 mx-0.5" />
            <Signal style={{ width: 11, height: 11 }} className="text-blue-400" />
            <span className="text-gray-300" style={{ fontSize: 11 }}>{quality}%</span>
            <div className="w-px h-3 bg-white/10 mx-0.5" />
            <span className="text-gray-400" style={{ fontSize: 11 }}>{formatTime(elapsed)}</span>
          </div>

          {/* Disconnect */}
          <button
            onClick={handleDisconnect}
            className="flex items-center gap-1 px-2 py-1 rounded-md bg-red-500/15 text-red-400 hover:bg-red-500/25 transition-colors"
            style={{ fontSize: 11 }}
          >
            <Power style={{ width: 11, height: 11 }} />
            断开
          </button>
          <button
            onClick={() => void handlePopOutWindow()}
            className="flex items-center gap-1 px-2 py-1 rounded-md bg-blue-500/15 text-blue-300 hover:bg-blue-500/25 transition-colors"
            style={{ fontSize: 11 }}
          >
            <Monitor style={{ width: 11, height: 11 }} />
            独立窗口
          </button>

          {/* Window controls */}
          <div className="flex items-center h-full ml-1">
            <button
              onClick={handleMinimize}
              className="flex items-center justify-center w-9 h-8 text-gray-500 hover:bg-white/8 hover:text-gray-300 transition-colors rounded-sm"
            >
              <Minus style={{ width: 14, height: 14 }} />
            </button>
            <button
              onClick={() => void handleToggleMaximize()}
              className="flex items-center justify-center w-9 h-8 text-gray-500 hover:bg-white/8 hover:text-gray-300 transition-colors rounded-sm"
              title={isMaximized ? "向下还原" : "最大化"}
            >
              {isMaximized ? (
                <Minimize2 style={{ width: 11, height: 11 }} />
              ) : (
                <Square style={{ width: 11, height: 11 }} />
              )}
            </button>
            <button
              onClick={handleCloseWindow}
              className="flex items-center justify-center w-9 h-8 text-gray-500 hover:bg-red-500 hover:text-white transition-colors rounded-sm"
              title="关闭"
            >
              <X style={{ width: 14, height: 14 }} />
            </button>
          </div>
        </div>
      </div>

      {/* Remote screen — full area */}
      <div className="flex-1 relative overflow-hidden cursor-crosshair select-none">
        {renderSnapshot?.preview_data_url ? (
          <img
            src={renderSnapshot.preview_data_url}
            alt="Remote desktop"
            className="w-full h-full object-contain bg-black"
            draggable={false}
          />
        ) : (
          <img
            src="https://images.unsplash.com/photo-1651832710372-a2b0da73a98f?crop=entropy&cs=tinysrgb&fit=max&fm=jpg&ixid=M3w3Nzg4Nzd8MHwxfHNlYXJjaHwxfHxyZW1vdGUlMjBkZXNrdG9wJTIwY29tcHV0ZXIlMjBzY3JlZW58ZW58MXx8fHwxNzcyNjE5MDE0fDA&ixlib=rb-4.1.0&q=80&w=1080"
            alt="Remote desktop"
            className="w-full h-full object-cover opacity-90"
            draggable={false}
          />
        )}

        {/* Connection quality overlay */}
        <div className="absolute top-3 right-3 flex items-center gap-2 px-2.5 py-1.5 rounded-lg bg-black/60 backdrop-blur-sm border border-white/10 text-gray-300" style={{ fontSize: 11 }}>
          <div className="w-1.5 h-1.5 rounded-full bg-green-400 animate-pulse" />
          连接稳定
        </div>

        {isWebRemoteSession ? (
          <div className="absolute top-3 left-3 w-80 max-w-[calc(100%-1.5rem)] rounded-lg bg-black/65 backdrop-blur-sm border border-white/10 text-gray-300">
            <div className="flex items-center justify-between px-3 py-2 border-b border-white/10">
              <div className="flex items-center gap-2">
                <Wifi style={{ width: 12, height: 12 }} className="text-emerald-300" />
                <span style={{ fontSize: 11 }}>WebRTC browser path</span>
              </div>
              <span
                className={
                  webRtcState === "connected"
                    ? "text-emerald-300"
                    : webRtcState === "failed"
                      ? "text-red-300"
                      : "text-amber-300"
                }
                style={{ fontSize: 11 }}
              >
                {webRtcState}
              </span>
            </div>
            <div className="px-3 py-2 space-y-1" style={{ fontSize: 11 }}>
              <div>{webRtcMessage}</div>
              <div className="text-gray-500">
                {webRemoteSession?.mode === "web_to_local" ? "web -> local" : "web -> peer"} / {id}
              </div>
            </div>
          </div>
        ) : null}

        {isTauriRuntime() ? (
          <div className="absolute top-3 left-3 w-72 max-w-[calc(100%-1.5rem)] rounded-lg bg-black/60 backdrop-blur-sm border border-white/10 text-gray-300">
            <div className="flex items-center justify-between px-3 py-2 border-b border-white/10">
              <div className="flex items-center gap-2">
                <Monitor style={{ width: 12, height: 12 }} className="text-blue-300" />
                <span style={{ fontSize: 11 }}>渲染窗口</span>
              </div>
              <span className="text-gray-400" style={{ fontSize: 11 }}>
                {currentRenderWindowCount ?? renderWindows.length}
              </span>
            </div>
            <div className="px-3 py-2 space-y-2">
              {currentRenderWindowLabel ? (
                <div className="rounded-md bg-blue-500/10 px-2 py-1.5 text-blue-200" style={{ fontSize: 11 }}>
                  <div>当前窗口: {currentRenderWindowLabel}</div>
                  <div className="mt-1 text-blue-100/80">
                    {currentSurfaceId ? `surface: ${currentSurfaceId}` : "surface: -"}
                    {currentWindowRole ? ` · role: ${currentWindowRole}` : ""}
                    {rendererAttached ? " · renderer attached" : " · renderer pending"}
                  </div>
                </div>
              ) : null}
              <div className="flex gap-2">
                <button
                  onClick={() => void handlePopOutWindow()}
                  className="rounded-md bg-white/8 px-2 py-1 text-gray-200 hover:bg-white/12"
                  style={{ fontSize: 10 }}
                >
                  新建 surface 窗口
                </button>
                <button
                  onClick={() => void handleOpenCurrentSurfaceWindow()}
                  disabled={!currentSurfaceId}
                  className="rounded-md bg-blue-500/15 px-2 py-1 text-blue-200 hover:bg-blue-500/25 disabled:cursor-not-allowed disabled:opacity-50"
                  style={{ fontSize: 10 }}
                >
                  复用当前 surface
                </button>
                <button
                  onClick={() => void handleOpenSelectedSurfaceWindow()}
                  disabled={!selectedSessionSurfaceId}
                  className="rounded-md bg-emerald-500/15 px-2 py-1 text-emerald-200 hover:bg-emerald-500/25 disabled:cursor-not-allowed disabled:opacity-50"
                  style={{ fontSize: 10 }}
                >
                  打开已选 surface
                </button>
                <button
                  onClick={() => void handleBindCurrentWindowSurface()}
                  disabled={!selectedSessionSurfaceId || selectedSessionSurfaceId === currentSurfaceId}
                  className="rounded-md bg-amber-500/15 px-2 py-1 text-amber-200 hover:bg-amber-500/25 disabled:cursor-not-allowed disabled:opacity-50"
                  style={{ fontSize: 10 }}
                >
                  绑定当前窗口
                </button>
              </div>
              <div className="rounded-md bg-white/6 px-2 py-2">
                <div className="mb-2 text-gray-400" style={{ fontSize: 10 }}>
                  Session Surfaces
                </div>
                <div className="flex gap-2">
                  <input
                    value={newSurfaceName}
                    onChange={(event) => setNewSurfaceName(event.target.value)}
                    placeholder="新 surface 名称"
                    className="min-w-0 flex-1 rounded-md border border-white/10 bg-black/20 px-2 py-1 text-gray-200 outline-none"
                    style={{ fontSize: 10 }}
                  />
                  <button
                    onClick={() => void handleCreateSurface()}
                    className="rounded-md bg-white/8 px-2 py-1 text-gray-200 hover:bg-white/12"
                    style={{ fontSize: 10 }}
                  >
                    创建
                  </button>
                </div>
                <div className="mt-2 space-y-1">
                  {renderSurfaces.length > 0 ? (
                    renderSurfaces.map((surface) => (
                      <button
                        key={surface.surface_id}
                        onClick={() => void handleSelectSurface(surface.surface_id)}
                        className={`flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left ${
                          surface.current || selectedSessionSurfaceId === surface.surface_id
                            ? "bg-blue-500/15 text-blue-200"
                            : "bg-white/6 text-gray-300 hover:bg-white/10"
                        }`}
                        style={{ fontSize: 10 }}
                      >
                        <span className="truncate">
                          {surface.name} · {surface.surface_id}
                        </span>
                        <span className="ml-2 shrink-0 text-[10px] text-gray-400">
                          {surface.current ? "current" : surface.role}
                        </span>
                      </button>
                    ))
                  ) : (
                    <div className="text-gray-500" style={{ fontSize: 10 }}>
                      当前会话还没有显式 surface
                    </div>
                  )}
                </div>
              </div>
              <div className="rounded-md bg-white/6 px-2 py-2">
                <div className="mb-2 text-gray-400" style={{ fontSize: 10 }}>
                  Surface Sources
                </div>
                <div className="flex gap-2">
                  <select
                    value={selectedSourceId ?? ""}
                    onChange={(event) => setSelectedSourceId(event.target.value || null)}
                    className="min-w-0 flex-1 rounded-md border border-white/10 bg-black/20 px-2 py-1 text-gray-200 outline-none"
                    style={{ fontSize: 10 }}
                  >
                    <option value="">选择 source</option>
                    {renderSnapshot?.available_source_ids.map((sourceId) => (
                      <option key={sourceId} value={sourceId}>
                        {sourceId}
                      </option>
                    ))}
                  </select>
                  <button
                    onClick={() => void handleBindSurfaceSource()}
                    disabled={!selectedSessionSurfaceId || !selectedSourceId}
                    className="rounded-md bg-fuchsia-500/15 px-2 py-1 text-fuchsia-200 hover:bg-fuchsia-500/25 disabled:cursor-not-allowed disabled:opacity-50"
                    style={{ fontSize: 10 }}
                  >
                    绑定 source
                  </button>
                </div>
                <div className="mt-2 space-y-1">
                  {renderSnapshot?.surface_source_bindings.length ? (
                    renderSnapshot.surface_source_bindings.map((binding) => (
                      <div
                        key={`${binding.surface_id}-${binding.source_id}`}
                        className="rounded-md bg-white/6 px-2 py-1.5 text-gray-300"
                        style={{ fontSize: 10 }}
                      >
                        {binding.surface_id} · {binding.source_id}
                      </div>
                    ))
                  ) : (
                    <div className="text-gray-500" style={{ fontSize: 10 }}>
                      当前还没有显式 source 绑定
                    </div>
                  )}
                </div>
              </div>
              <div className="rounded-md bg-white/6 px-2 py-2">
                <div className="mb-2 flex items-center justify-between gap-2">
                  <span className="text-gray-400" style={{ fontSize: 10 }}>
                    NVDEC Capability
                  </span>
                  <span className="text-gray-500" style={{ fontSize: 10 }}>
                    {nvdecProbe?.backend ?? "windows-nvdec"}
                  </span>
                </div>
                <div
                  className="rounded-md bg-black/20 px-2 py-1.5 text-gray-300"
                  style={{ fontSize: 10 }}
                >
                  {nvdecProbe?.summary ?? "未读取 NVDEC 状态"}
                </div>
                <div className="mt-2 flex items-center justify-between gap-2 rounded-md bg-white/6 px-2 py-1.5">
                  <div>
                    <div className="text-gray-400" style={{ fontSize: 10 }}>
                      Policy
                    </div>
                    <div className="mt-1 text-gray-200" style={{ fontSize: 10 }}>
                      {decodePolicy?.decode_policy ?? "auto"}
                    </div>
                  </div>
                  <select
                    value={decodePolicy?.decode_policy ?? "auto"}
                    onChange={(event) =>
                      void handleUpdateDecodePolicy(event.target.value as DecoderPolicy)
                    }
                    className="rounded-md border border-white/10 bg-black/20 px-2 py-1 text-gray-200 outline-none"
                    style={{ fontSize: 10 }}
                  >
                    <option value="auto">auto</option>
                    <option value="software">software</option>
                    <option value="nvdec">nvdec</option>
                  </select>
                </div>
                <div className="mt-2 grid grid-cols-2 gap-2">
                  <div
                    className="rounded-md bg-white/6 px-2 py-1.5 text-gray-300"
                    style={{ fontSize: 10 }}
                  >
                    <div className="text-gray-500">Preferred</div>
                    <div className="mt-1">
                      {webrtcHostSnapshot?.preferredDecodeBackend ?? "未选择"}
                    </div>
                  </div>
                  <div
                    className="rounded-md bg-white/6 px-2 py-1.5 text-gray-300"
                    style={{ fontSize: 10 }}
                  >
                    <div className="text-gray-500">Active</div>
                    <div className="mt-1">
                      {webrtcHostSnapshot?.activeDecodeBackend ?? "未激活"}
                    </div>
                  </div>
                </div>
                <div
                  className="mt-2 rounded-md bg-white/6 px-2 py-1.5 text-gray-400"
                  style={{ fontSize: 10 }}
                >
                  {webrtcHostSnapshot?.decodeBackendReason ?? "当前会话还没有 decoder 选择信息"}
                </div>
                <div className="mt-2 grid grid-cols-2 gap-2">
                  <div
                    className="rounded-md bg-white/6 px-2 py-1.5 text-gray-300"
                    style={{ fontSize: 10 }}
                  >
                    <div className="text-gray-500">Fallbacks</div>
                    <div className="mt-1">
                      {webrtcHostSnapshot?.decodeFallbackCount ?? 0}
                    </div>
                  </div>
                  <div
                    className="rounded-md bg-white/6 px-2 py-1.5 text-gray-300"
                    style={{ fontSize: 10 }}
                  >
                    <div className="text-gray-500">Policy</div>
                    <div className="mt-1">
                      {webrtcHostSnapshot?.decodePolicy ?? decodePolicy?.decode_policy ?? "auto"}
                    </div>
                  </div>
                </div>
                <div
                  className="mt-2 rounded-md bg-white/6 px-2 py-1.5 text-gray-400"
                  style={{ fontSize: 10 }}
                >
                  {webrtcHostSnapshot?.lastDecodeFallbackReason ?? "当前会话没有 fallback 记录"}
                </div>
                <div className="mt-2 space-y-1">
                  {[
                    { label: "H264", codec: "h264", bitDepthMinus8: 0 },
                    { label: "HEVC", codec: "hevc", bitDepthMinus8: 0 },
                    { label: "Main10", codec: "hevc", bitDepthMinus8: 2 },
                  ].map((item) => {
                    const capability = nvdecProbe?.capability_probes.find(
                      (probe) =>
                        probe.codec === item.codec &&
                        probe.bit_depth_minus8 === item.bitDepthMinus8,
                    );
                    return (
                      <div
                        key={`${item.codec}-${item.bitDepthMinus8}`}
                        className="rounded-md bg-white/6 px-2 py-1.5 text-gray-300"
                        style={{ fontSize: 10 }}
                      >
                        <div className="flex items-center justify-between gap-2">
                          <span>{item.label}</span>
                          <span
                            className={
                              capability?.runtime_supported
                                ? "text-green-300"
                                : "text-amber-300"
                            }
                          >
                            {capability?.runtime_supported ? "runtime" : "no-runtime"}
                          </span>
                        </div>
                        <div className="mt-1 text-gray-500">
                          {capability?.wired_supported ? "已接线" : capability?.wired_reason ?? "未读取"}
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
              {renderWindows.length > 0 ? (
                renderWindows.map((window) => (
                  <div
                    key={window.label}
                    className="flex items-center justify-between gap-2 rounded-md bg-white/6 px-2 py-1.5"
                  >
                    <div className="min-w-0">
                      <div className="truncate text-gray-300" style={{ fontSize: 11 }}>
                        {window.label}
                      </div>
                      <div className="truncate text-gray-500" style={{ fontSize: 10 }}>
                        {window.surface_id} · {window.role} · {window.renderer_attached ? "attached" : "pending"}
                      </div>
                    </div>
                    <button
                      onClick={() => void handleCloseRenderWindow(window.label)}
                      className="rounded-md px-2 py-1 text-red-300 hover:bg-red-500/15"
                      style={{ fontSize: 10 }}
                    >
                      关闭
                    </button>
                  </div>
                ))
              ) : (
                <div className="text-gray-500" style={{ fontSize: 11 }}>
                  当前会话还没有独立渲染窗口
                </div>
              )}
            </div>
          </div>
        ) : null}

        {/* Device info badge */}
        <div className="absolute bottom-3 left-3 px-2.5 py-1.5 rounded-lg bg-black/60 backdrop-blur-sm border border-white/10 text-gray-400" style={{ fontSize: 11 }}>
          {device.name} · {device.os} · {renderSnapshot?.frame ? `${renderSnapshot.frame.width}×${renderSnapshot.frame.height}` : "1920×1080"}
        </div>

        <div className="absolute bottom-3 right-3 px-2.5 py-1.5 rounded-lg bg-black/60 backdrop-blur-sm border border-white/10 text-gray-400" style={{ fontSize: 11 }}>
          {renderSnapshot?.renderer_backend
            ? `${renderSnapshot.renderer_backend} · ${renderSnapshot.surface_count} surfaces · ${renderSnapshot.renderer_snapshot?.uploaded_frame_count ?? 0} uploads`
            : "renderer idle"}
        </div>
      </div>

      {/* Status bar */}
      <div className="flex items-center justify-between px-4 py-1.5 bg-[#232340] border-t border-white/10 shrink-0">
        <div className="flex items-center gap-4">
          <StatusItem
            label="分辨率"
            value={renderSnapshot?.frame ? `${renderSnapshot.frame.width}×${renderSnapshot.frame.height}` : "1920×1080"}
          />
          <StatusItem label="帧率" value="60 fps" />
          <StatusItem
            label="帧缓冲"
            value={renderSnapshot?.frame ? `${Math.round(renderSnapshot.frame.bytes / 1024)} KB` : "-"}
          />
        </div>
        <div className="flex items-center gap-1 text-green-400" style={{ fontSize: 11 }}>
          <Lock style={{ width: 11, height: 11 }} />
          TLS 1.3 加密
        </div>
      </div>
    </div>
  );
}

function CtrlBtn({
  icon, label, onClick, active, danger,
}: {
  icon: React.ReactNode; label: string; onClick?: () => void; active?: boolean; danger?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      title={label}
      className={`flex items-center gap-1 px-1.5 py-1 rounded-md transition-colors ${
        danger
          ? "text-red-400/70 hover:bg-red-500/15 hover:text-red-400"
          : active === false
          ? "text-gray-500 hover:bg-white/6 hover:text-gray-300"
          : "text-gray-400 hover:bg-white/8 hover:text-gray-200"
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
