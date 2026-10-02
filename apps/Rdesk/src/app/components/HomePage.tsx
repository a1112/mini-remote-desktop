import { useState, useEffect, useRef } from "react";
import { useNavigate } from "react-router";
import {
  Monitor,
  Copy,
  ArrowRight,
  Clock,
  ChevronRight,
  X,
  Check,
  Loader2,
  Pencil,
  AlertCircle,
} from "lucide-react";
import { useTheme } from "./ThemeContext";
import { useDeviceRegistration, deviceService } from "../services/deviceService";
import { launchRemoteDisplayForDevice } from "../services/remoteDisplayLauncher";
import { useDevices } from "./deviceData";
import { useConnectionHistory } from "../services/connectionHistoryService";
import { DeviceRegisterModal } from "./DeviceRegisterModal";

export function HomePage() {
  const { isDark } = useTheme();
  const navigate = useNavigate();
  const { devices } = useDevices();
  const history = useConnectionHistory();
  const recentConnections = history
    .filter((entry, index, entries) => entries.findIndex((item) => item.peerDeviceId === entry.peerDeviceId) === index)
    .slice(0, 4)
    .map((entry) => {
      const device = devices.find((candidate) => candidate.deviceId === entry.peerDeviceId);
      return {
        id: device?.id ?? entry.peerDeviceId,
        name: device?.name ?? entry.peerDeviceId,
        deviceId: entry.peerDeviceId,
        os: device?.os ?? "",
        icon: device?.icon ?? Monitor,
        status: device?.status,
        lastConnected: new Date(entry.startedAt).toLocaleString("zh-CN"),
      };
    });
  const { deviceId: myDeviceId, deviceName: myDeviceName, registrationError, refresh } = useDeviceRegistration();
  const [registrationOpen, setRegistrationOpen] = useState(false);
  const [connectId, setConnectId] = useState("");
  const [copied, setCopied] = useState(false);
  const [connectingDeviceId, setConnectingDeviceId] = useState<string | null>(null);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const connectionInFlightRef = useRef(false);

  // 设备名称编辑状态
  const [editingDeviceName, setEditingDeviceName] = useState(false);
  const [deviceNameEdit, setDeviceNameEdit] = useState("");
  const deviceInputRef = useRef<HTMLInputElement>(null);

  const handleCopy = () => {
    if (!myDeviceId) return;
    navigator.clipboard.writeText(myDeviceId.replace(/\s/g, ""));
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  // 开始编辑设备名称
  const handleStartEditDeviceName = () => {
    setEditingDeviceName(true);
    setDeviceNameEdit(myDeviceName || "未命名设备");
    setTimeout(() => deviceInputRef.current?.focus(), 0);
  };

  // 确认编辑设备名称
  const handleConfirmEditDeviceName = async () => {
    if (deviceNameEdit.trim() && myDeviceId) {
      const success = await deviceService.renameDevice(myDeviceId, deviceNameEdit.trim());
      if (success) {
        // 刷新设备信息会通过重新获取来更新
        window.location.reload(); // 简单刷新页面来更新
      }
    }
    setEditingDeviceName(false);
  };

  // 取消编辑设备名称
  const handleCancelEditDeviceName = () => {
    setEditingDeviceName(false);
    setDeviceNameEdit("");
  };

  // 设备名称键盘事件
  const handleDeviceNameKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      handleConfirmEditDeviceName();
    } else if (e.key === "Escape") {
      e.preventDefault();
      handleCancelEditDeviceName();
    }
  };

  // 格式化设备 ID（每3位一组）
  const formatDeviceId = (id: string | null) => {
    if (!id) return "设备 ID 未就绪";
    const cleaned = id.replace(/\s/g, "");
    if (cleaned.length === 9) {
      return `${cleaned.slice(0, 3)} ${cleaned.slice(3, 6)} ${cleaned.slice(6)}`;
    }
    return cleaned;
  };

  const myId = formatDeviceId(myDeviceId);

  // 自动聚焦设备名称输入框
  useEffect(() => {
    if (editingDeviceName) {
      deviceInputRef.current?.focus();
      deviceInputRef.current?.select();
    }
  }, [editingDeviceName]);

  const launchSecureRemote = async (target: {
    id: string;
    name: string;
    deviceId: string;
    os: string;
  }) => {
    if (connectionInFlightRef.current) return;
    connectionInFlightRef.current = true;
    const targetDeviceId = target.deviceId.replace(/\s/g, "");
    setConnectingDeviceId(target.id);
    setConnectionError(null);
    try {
      const result = await launchRemoteDisplayForDevice(targetDeviceId, {
        transportKind: "webrtc",
        targetDeviceName: target.name,
        targetOs: target.os,
        routePreference: "auto",
      });
      navigate(`/session/${result.sessionId}`);
    } catch (error) {
      setConnectionError(
        error instanceof Error ? error.message : "安全远程会话请求失败",
      );
    } finally {
      connectionInFlightRef.current = false;
      setConnectingDeviceId(null);
    }
  };

  const handleConnect = () => {
    if (!connectId.trim()) return;
    const found = devices.find((device) => device.deviceId.replace(/\s/g, "") === connectId.replace(/\s/g, ""));
    if (found) {
      void launchSecureRemote(found);
    } else {
      void launchSecureRemote({
        id: "custom",
        name: "远程设备",
        deviceId: connectId,
        os: "Unknown",
      });
    }
  };

  // Reusable dark-aware classes
  const card = isDark ? "bg-[#232323] border-gray-700" : "bg-white border-gray-200/70 shadow-sm";
  const textPrimary = isDark ? "text-gray-100" : "text-gray-900";
  const textSecondary = isDark ? "text-gray-400" : "text-gray-500";
  const textTertiary = isDark ? "text-gray-500" : "text-gray-400";
  const textBody = isDark ? "text-gray-300" : "text-gray-700";
  const inputBg = isDark
    ? "bg-[#2a2a2a] border-gray-600 text-gray-200 placeholder-gray-500 focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20"
    : "bg-gray-50 border-gray-200 text-gray-900 placeholder-gray-400 focus:border-blue-400 focus:ring-2 focus:ring-blue-100";
  const btnSecondary = isDark
    ? "border-gray-600 text-gray-400 hover:bg-gray-800 hover:text-gray-200"
    : "border-gray-200 text-gray-500 hover:bg-gray-50 hover:text-gray-800";

  return (
    <div className="p-8 max-w-6xl mx-auto">
      <DeviceRegisterModal
        isOpen={registrationOpen}
        onClose={() => setRegistrationOpen(false)}
        onSuccess={() => refresh()}
      />
      <div className="grid grid-cols-5 gap-6">
        {/* Left: ID + Connect */}
        <div className="col-span-2 space-y-5">
          {/* My Device card */}
          <div className={`p-5 rounded-xl border ${card}`}>
            {/* 设备名称 + 编辑按钮 */}
            <div className="flex items-center justify-center gap-2 mb-2">
              {editingDeviceName ? (
                <div className="flex items-center gap-2 w-full">
                  <input
                    ref={deviceInputRef}
                    type="text"
                    value={deviceNameEdit}
                    onChange={(e) => setDeviceNameEdit(e.target.value)}
                    onKeyDown={handleDeviceNameKeyDown}
                    className={`flex-1 px-2 py-1 rounded-md border text-center outline-none transition-colors ${
                      isDark
                        ? "bg-[#1a1a1a] border-blue-500 text-gray-200 placeholder-gray-500 focus:ring-1 focus:ring-blue-500"
                        : "bg-white border-blue-400 text-gray-900 placeholder-gray-400 focus:ring-1 focus:ring-blue-400"
                    }`}
                    style={{ fontSize: 15 }}
                  />
                  <button
                    onClick={handleConfirmEditDeviceName}
                    disabled={!deviceNameEdit.trim()}
                    className={`px-2 py-1 rounded-md text-xs font-medium transition-colors ${
                      !deviceNameEdit.trim()
                        ? isDark
                          ? "bg-gray-700 text-gray-500 cursor-not-allowed"
                          : "bg-gray-100 text-gray-400 cursor-not-allowed"
                        : isDark
                          ? "bg-blue-600 text-white hover:bg-blue-500"
                          : "bg-blue-600 text-white hover:bg-blue-500"
                    }`}
                  >
                    <Check className="w-3.5 h-3.5" />
                  </button>
                  <button
                    onClick={handleCancelEditDeviceName}
                    className={`p-1 rounded-md transition-colors ${isDark ? "text-gray-400 hover:bg-gray-700" : "text-gray-400 hover:bg-gray-100"}`}
                  >
                    <X className="w-3.5 h-3.5" />
                  </button>
                </div>
              ) : (
                <>
                  <span className={`text-lg font-medium ${textPrimary}`} style={{ fontSize: 16 }}>
                    {myDeviceName || "未命名设备"}
                  </span>
                  <button
                    onClick={handleStartEditDeviceName}
                    className={`p-1 rounded transition-colors ${isDark ? "text-gray-500 hover:text-gray-300" : "text-gray-400 hover:text-gray-600"}`}
                    title="编辑设备名称"
                  >
                    <Pencil className="w-3 h-3" />
                  </button>
                </>
              )}
            </div>

            {/* 设备ID + 复制图标 */}
            <div className="flex items-center justify-center gap-1.5 mb-4">
              <div className={`text-2xl font-mono tracking-widest ${textPrimary}`} style={{ fontSize: 18 }}>
                {myId}
              </div>
              <button
                onClick={handleCopy}
                disabled={!myDeviceId}
                className={`p-1 rounded transition-colors ${copied
                    ? "text-green-600"
                    : isDark ? "text-gray-500 hover:text-gray-300" : "text-gray-400 hover:text-gray-600"
                }`}
                title={copied ? "已复制" : "复制ID"}
              >
                <Copy className="w-3.5 h-3.5" />
              </button>
            </div>
            {registrationError && <p role="alert" className="mb-3 text-sm text-amber-600">{registrationError}</p>}
            <button onClick={() => setRegistrationOpen(true)} className={`w-full py-2 rounded-lg border text-sm ${btnSecondary}`}>
              登记到服务器
            </button>
          </div>

          {/* Connect card */}
          <div className={`p-5 rounded-xl border ${card}`}>
            <h3 className={`mb-4 ${textPrimary}`} style={{ fontSize: 15 }}>连接到远程设备</h3>

            <div className="space-y-3">
              <div>
                <label className={`block mb-1.5 ${textSecondary}`} style={{ fontSize: 12 }}>
                  远程设备 ID
                </label>
                <input
                  type="text"
                  value={connectId}
                  onChange={(e) => setConnectId(e.target.value)}
                  placeholder="输入远程设备 ID"
                  onKeyDown={(e) => e.key === "Enter" && handleConnect()}
                  className={`w-full px-3 py-2.5 rounded-lg border outline-none transition-all ${inputBg}`}
                  style={{ fontSize: 14 }}
                />
              </div>

              <div
                className={`rounded-lg border px-3 py-2.5 ${
                  isDark
                    ? "border-blue-500/20 bg-blue-500/10 text-blue-200"
                    : "border-blue-100 bg-blue-50 text-blue-700"
                }`}
              >
                <div className="font-medium" style={{ fontSize: 12 }}>
                  目标设备确认授权
                </div>
                <div className="mt-1 opacity-75" style={{ fontSize: 11 }}>
                  当前安全会话仅支持目标端现场确认；不会使用密码绕过确认。
                </div>
              </div>

              <button
                onClick={handleConnect}
                disabled={!connectId.trim() || connectingDeviceId !== null}
                className="w-full flex items-center justify-center gap-2 py-2.5 rounded-lg bg-blue-600 hover:bg-blue-500 disabled:opacity-40 disabled:cursor-not-allowed text-white transition-colors shadow-sm"
                style={{ fontSize: 14 }}
              >
                <span>立即连接</span>
                {connectingDeviceId === "custom" ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <ArrowRight className="w-4 h-4" />
                )}
              </button>
              {connectionError ? (
                <div
                  role="alert"
                  className="mt-3 flex items-start gap-2 rounded-md border border-red-500/25 bg-red-500/10 px-3 py-2 text-xs text-red-500"
                >
                  <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  <span>{connectionError}</span>
                </div>
              ) : null}
            </div>
          </div>
        </div>

        {/* Right: Recent connections */}
        <div className="col-span-3">
          <div className={`p-5 rounded-xl border h-full ${card}`}>
            <div className="flex items-center justify-between mb-4">
              <h3 className={textPrimary} style={{ fontSize: 15 }}>最近连接</h3>
              <button onClick={() => navigate("/connections")} className="flex items-center gap-1 text-blue-600 hover:text-blue-500 transition-colors" style={{ fontSize: 13 }}>
                查看全部 <ChevronRight className="w-3.5 h-3.5" />
              </button>
            </div>

            <div className="space-y-2">
              {recentConnections.map((device) => {
                const Icon = device.icon;
                return (
                  <div
                    key={device.id}
                    className={`flex items-center gap-3 p-3.5 rounded-lg border border-transparent transition-all group cursor-pointer ${
                      isDark
                        ? "bg-[#2a2a2a]/60 hover:bg-[#333] hover:border-gray-600"
                        : "bg-gray-50/60 hover:bg-gray-100 hover:border-gray-200"
                    }`}
                    onClick={() => void launchSecureRemote(device)}
                  >
                    <div className={`relative w-9 h-9 rounded-lg flex items-center justify-center shrink-0 ${
                      device.status === "online"
                        ? isDark ? "bg-blue-900/30" : "bg-blue-50"
                        : isDark ? "bg-gray-800" : "bg-gray-100"
                    }`}>
                      <Icon className={`w-4.5 h-4.5 ${device.status === "online" ? "text-blue-600" : "text-gray-400"}`} style={{ width: 18, height: 18 }} />
                      {device.status && <div className={`absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full border-2 ${
                        isDark ? "border-[#232323]" : "border-white"
                      } ${device.status === "online" ? "bg-green-500" : "bg-gray-300"}`} />}
                    </div>

                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className={`font-medium truncate ${isDark ? "text-gray-200" : "text-gray-800"}`} style={{ fontSize: 14 }}>
                          {device.name}
                        </span>
                      </div>
                      <div className="flex items-center gap-2 mt-0.5">
                        <span className={`font-mono ${textTertiary}`} style={{ fontSize: 11 }}>{device.deviceId}</span>
                        {device.os && <span className={textTertiary} style={{ fontSize: 11 }}>· {device.os}</span>}
                      </div>
                    </div>

                    <div className="flex items-center gap-3 text-right">
                      <span className={textTertiary} style={{ fontSize: 12 }}>{device.lastConnected}</span>
                      {(
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            void launchSecureRemote(device);
                          }}
                          disabled={connectingDeviceId !== null}
                          className={`flex items-center gap-1 px-2.5 py-1 rounded-md transition-colors opacity-0 group-hover:opacity-100 ${
                            isDark ? "bg-blue-900/30 text-blue-400 hover:bg-blue-900/50" : "bg-blue-50 text-blue-600 hover:bg-blue-100"
                          }`}
                          style={{ fontSize: 12 }}
                        >
                          连接
                          <ArrowRight className="w-3 h-3" />
                        </button>
                      )}
                    </div>
                  </div>
                );
              })}
              {recentConnections.length === 0 && <div className={`py-16 text-center text-sm ${textSecondary}`}>暂无真实连接记录</div>}
            </div>

            {/* Quick tips */}
            <div className={`mt-4 p-3 rounded-lg border flex items-start gap-3 ${
              isDark ? "bg-blue-900/20 border-blue-800" : "bg-blue-50 border-blue-100"
            }`}>
              <Clock className="w-4 h-4 text-blue-600 shrink-0 mt-0.5" />
              <div>
                <div className={isDark ? "text-blue-400" : "text-blue-700"} style={{ fontSize: 13 }}>提示</div>
                <div className={`mt-0.5 ${textSecondary}`} style={{ fontSize: 12 }}>
                  连接记录仅在远程会话实际开始后显示。点击设备可以再次发起连接。
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
