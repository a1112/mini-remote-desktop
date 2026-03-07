import { useState, useEffect } from "react";
import {
  X,
  Monitor,
  Cpu,
  HardDrive,
  MemoryStick,
  CheckCircle,
  AlertCircle,
  Loader2,
} from "lucide-react";
import { useTheme } from "./ThemeContext";

interface HardwareInfo {
  motherboard_serial: string;
  hostname: string;
  os_type: string;
  os_version: string;
  cpu_info: {
    name: string;
    vendor_id: string;
    cores: number;
    max_frequency_mhz?: number;
  };
  total_memory_mb: number;
  gpu_info: Array<{
    name: string;
    vendor: string;
    memory_mb?: number;
  }>;
}

interface DeviceRegisterResponse {
  device_id: string;
  device_name: string;
  access_token: string;
}

interface DeviceRegisterModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSuccess?: (deviceId: string, deviceName: string, accessToken: string) => void;
}

export function DeviceRegisterModal({
  isOpen,
  onClose,
  onSuccess,
}: DeviceRegisterModalProps) {
  const { isDark } = useTheme();
  const [step, setStep] = useState<"loading" | "register" | "success">("loading");
  const [hardwareInfo, setHardwareInfo] = useState<HardwareInfo | null>(null);
  const [deviceName, setDeviceName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [registering, setRegistering] = useState(false);
  const [result, setResult] = useState<DeviceRegisterResponse | null>(null);

  // 获取硬件信息
  useEffect(() => {
    if (isOpen) {
      fetchHardwareInfo();
    }
  }, [isOpen]);

  const fetchHardwareInfo = async () => {
    try {
      if (window.__TAURI__) {
        const info = await window.__TAURI__.invoke<HardwareInfo>("get_hardware_info");
        setHardwareInfo(info);
        setDeviceName(info.hostname);
        setStep("register");
      } else {
        // 开发模式模拟数据
        const mockInfo: HardwareInfo = {
          motherboard_serial: "MOCK-" + Math.random().toString(36).substring(7),
          hostname: "开发测试机",
          os_type: "windows",
          os_version: "Windows 11 Pro 23H2 Build 22631",
          cpu_info: {
            name: "Intel Core i7-12700K",
            vendor_id: "GenuineIntel",
            cores: 12,
            max_frequency_mhz: 3500,
          },
          total_memory_mb: 32768,
          gpu_info: [
            {
              name: "NVIDIA GeForce RTX 3060",
              vendor: "NVIDIA",
              memory_mb: 12288,
            },
          ],
        };
        setHardwareInfo(mockInfo);
        setDeviceName(mockInfo.hostname);
        setStep("register");
      }
    } catch (err) {
      setError(`获取硬件信息失败: ${err}`);
      setStep("register");
    }
  };

  const handleRegister = async () => {
    if (!hardwareInfo) return;

    setRegistering(true);
    setError(null);

    try {
      let response: DeviceRegisterResponse;

      if (window.__TAURI__) {
        response = await window.__TAURI__.invoke<DeviceRegisterResponse>("register_device", {
          motherboard_serial: hardwareInfo.motherboard_serial,
          hostname: hardwareInfo.hostname,
          os_version: hardwareInfo.os_version,
          deviceName: deviceName || hardwareInfo.hostname,
        });
      } else {
        // 开发模式模拟注册
        await new Promise((resolve) => setTimeout(resolve, 1500));
        response = {
          device_id: "DEV-" + Math.random().toString(36).substring(7).toUpperCase(),
          device_name: deviceName || hardwareInfo.hostname,
          access_token: "mock-access-token-" + Math.random().toString(36).substring(7),
        };
      }

      setResult(response);
      setStep("success");

      if (onSuccess) {
        onSuccess(response.device_id, response.device_name, response.access_token);
      }
    } catch (err) {
      setError(`注册失败: ${err}`);
    } finally {
      setRegistering(false);
    }
  };

  const handleClose = () => {
    setStep("loading");
    setHardwareInfo(null);
    setDeviceName("");
    setError(null);
    setResult(null);
    onClose();
  };

  if (!isOpen) return null;

  const cardBg = isDark ? "bg-[#232323]" : "bg-white";
  const textPrimary = isDark ? "text-gray-100" : "text-gray-900";
  const textSecondary = isDark ? "text-gray-400" : "text-gray-500";
  const inputBg = isDark
    ? "bg-[#2a2a2a] border-gray-600 text-gray-200"
    : "bg-gray-50 border-gray-200 text-gray-900";
  const buttonPrimary = isDark
    ? "bg-blue-600 hover:bg-blue-500 text-white"
    : "bg-blue-600 hover:bg-blue-700 text-white";
  const buttonDisabled = isDark
    ? "bg-gray-700 text-gray-500 cursor-not-allowed"
    : "bg-gray-200 text-gray-400 cursor-not-allowed";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Overlay */}
      <div
        className={`absolute inset-0 ${isDark ? "bg-black/60" : "bg-black/40"}`}
        onClick={handleClose}
      />

      {/* Modal */}
      <div
        className={`relative w-full max-w-lg rounded-2xl shadow-2xl ${cardBg} border ${
          isDark ? "border-gray-700" : "border-gray-200"
        }`}
      >
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b">
          <h2 className={`text-xl font-semibold ${textPrimary}`}>
            {step === "success" ? "设备注册成功" : "设备注册"}
          </h2>
          <button
            onClick={handleClose}
            className={`p-2 rounded-lg ${isDark ? "hover:bg-gray-700" : "hover:bg-gray-100"}`}
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Content */}
        <div className="p-6">
          {step === "loading" && (
            <div className="flex flex-col items-center justify-center py-12">
              <Loader2 className="w-12 h-12 animate-spin text-blue-500 mb-4" />
              <p className={textSecondary}>正在获取硬件信息...</p>
            </div>
          )}

          {step === "register" && hardwareInfo && (
            <div className="space-y-6">
              {/* Hardware Info Display */}
              <div className={`p-4 rounded-xl ${isDark ? "bg-[#2a2a2a]" : "bg-gray-50"}`}>
                <div className="flex items-center gap-3 mb-4">
                  <Monitor className="w-6 h-6 text-blue-500" />
                  <div>
                    <div className={`text-sm ${textSecondary}`}>设备标识</div>
                    <div className={`font-mono text-sm ${textPrimary}`}>
                      {hardwareInfo.motherboard_serial}
                    </div>
                  </div>
                </div>

                <div className="grid grid-cols-2 gap-4 text-sm">
                  <div>
                    <div className={textSecondary}>主机名</div>
                    <div className={textPrimary}>{hardwareInfo.hostname}</div>
                  </div>
                  <div>
                    <div className={textSecondary}>操作系统</div>
                    <div className={textPrimary}>{hardwareInfo.os_version}</div>
                  </div>
                  <div>
                    <div className={textSecondary}>CPU</div>
                    <div className={textPrimary}>{hardwareInfo.cpu_info.name}</div>
                  </div>
                  <div>
                    <div className={textSecondary}>内存</div>
                    <div className={textPrimary}>
                      {(hardwareInfo.total_memory_mb / 1024).toFixed(1)} GB
                    </div>
                  </div>
                </div>

                {hardwareInfo.gpu_info.length > 0 && (
                  <div className="mt-4">
                    <div className={textSecondary}>显卡</div>
                    <div className={textPrimary}>
                      {hardwareInfo.gpu_info.map((gpu, i) => (
                        <div key={i} className="text-sm">
                          {gpu.name} {gpu.memory_mb && `(${gpu.memory_mb / 1024}GB)`}
                        </div>
                      ))}
                    </div>
                  </div>
                )}
              </div>

              {/* Device Name Input */}
              <div>
                <label className={`block text-sm font-medium mb-2 ${textPrimary}`}>
                  设备名称
                </label>
                <input
                  type="text"
                  value={deviceName}
                  onChange={(e) => setDeviceName(e.target.value)}
                  placeholder="输入设备显示名称"
                  className={`w-full px-4 py-3 rounded-lg border ${inputBg} focus:outline-none focus:ring-2 focus:ring-blue-500`}
                />
              </div>

              {/* Error Message */}
              {error && (
                <div className={`flex items-start gap-3 p-4 rounded-lg ${
                  isDark ? "bg-red-900/20 text-red-400" : "bg-red-50 text-red-600"
                }`}>
                  <AlertCircle className="w-5 h-5 flex-shrink-0 mt-0.5" />
                  <span className="text-sm">{error}</span>
                </div>
              )}

              {/* Register Button */}
              <button
                onClick={handleRegister}
                disabled={registering || !deviceName.trim()}
                className={`w-full py-3 rounded-lg font-medium transition-colors ${
                  registering || !deviceName.trim()
                    ? buttonDisabled
                    : buttonPrimary
                }`}
              >
                {registering ? (
                  <span className="flex items-center justify-center gap-2">
                    <Loader2 className="w-5 h-5 animate-spin" />
                    注册中...
                  </span>
                ) : (
                  "注册设备"
                )}
              </button>
            </div>
          )}

          {step === "success" && result && (
            <div className="text-center py-8">
              <div className="flex justify-center mb-6">
                <div className={`w-16 h-16 rounded-full flex items-center justify-center ${
                  isDark ? "bg-green-900/30" : "bg-green-100"
                }`}>
                  <CheckCircle className="w-10 h-10 text-green-500" />
                </div>
              </div>

              <h3 className={`text-xl font-semibold mb-2 ${textPrimary}`}>
                设备注册成功！
              </h3>

              <div className={`my-6 p-4 rounded-xl ${isDark ? "bg-[#2a2a2a]" : "bg-gray-50"}`}>
                <div className="grid grid-cols-2 gap-4 text-left">
                  <div>
                    <div className={`text-sm ${textSecondary}`}>设备 ID</div>
                    <div className={`font-mono ${textPrimary}`}>{result.device_id}</div>
                  </div>
                  <div>
                    <div className={`text-sm ${textSecondary}`}>设备名称</div>
                    <div className={textPrimary}>{result.device_name}</div>
                  </div>
                </div>
              </div>

              <p className={`text-sm ${textSecondary} mb-6`}>
                设备已成功注册到服务器，现在可以开始使用远程桌面功能。
              </p>

              <button
                onClick={handleClose}
                className={`px-8 py-3 rounded-lg font-medium ${buttonPrimary}`}
              >
                完成
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// 扩展 window 类型以支持 Tauri invoke
declare global {
  interface Window {
    __TAURI__?: {
      invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
    };
  }
}
