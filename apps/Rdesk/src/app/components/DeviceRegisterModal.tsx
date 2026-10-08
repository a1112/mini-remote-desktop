import { useEffect, useRef, useState } from "react";
import { X, CheckCircle, Loader2, RefreshCw, AlertCircle } from "lucide-react";
import { useTheme } from "./ThemeContext";
import { useDeviceRegistration, usePublicServerStatus } from "../services/deviceService";
import { deviceCodeLabel, formatDeviceCode } from "../utils/deviceCode";
import { LOCAL_SERVICE_STOPPED_MESSAGE, LOCAL_SERVICE_UNREACHABLE_MESSAGE } from "../utils/automaticDeviceEnrollment";

interface DeviceRegisterModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSuccess?: () => void;
}

export function DeviceRegisterModal({ isOpen, ...props }: DeviceRegisterModalProps) {
  return isOpen ? <AutomaticDeviceRegistrationDialog {...props} /> : null;
}

function AutomaticDeviceRegistrationDialog({ onClose, onSuccess }: Omit<DeviceRegisterModalProps, "isOpen">) {
  const { isDark } = useTheme();
  const { deviceId: managedDeviceId, registrationError } = useDeviceRegistration();
  const { status, checking, failed, starting, startupError, refresh, startService } = usePublicServerStatus();
  const [retrying, setRetrying] = useState(false);
  const notifiedDeviceId = useRef<string | null>(null);
  const deviceId = status?.service_running && status.device_registered && status.device_id === managedDeviceId
    ? managedDeviceId : null;

  useEffect(() => {
    if (deviceId && notifiedDeviceId.current !== deviceId) {
      notifiedDeviceId.current = deviceId;
      onSuccess?.();
    }
  }, [deviceId, onSuccess]);

  const retry = async () => {
    setRetrying(true);
    try { await refresh(); } finally { setRetrying(false); }
  };
  const localUnavailable = failed || status?.service_running === false;
  const message = starting ? "正在启动本机后台服务…"
    : startupError ?? (failed ? LOCAL_SERVICE_UNREACHABLE_MESSAGE
      : status?.service_running === false ? LOCAL_SERVICE_STOPPED_MESSAGE
      : registrationError ?? (status ? "正在自动登记并领取设备码，请稍候。" : "正在检查本机后台服务…"));
  const inProgress = starting || (!localUnavailable && (
    checking || status?.last_error === "public_auto_enrollment_pending"
    || Boolean(status?.service_running && !status.device_registered && !status.last_error && status.api_url && status.api_reachable !== false)
  ));
  const card = isDark ? "bg-[#232323] border-gray-700 text-gray-100" : "bg-white border-gray-200 text-gray-900";
  const secondary = isDark ? "text-gray-400" : "text-gray-500";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className={`absolute inset-0 ${isDark ? "bg-black/60" : "bg-black/40"}`} onClick={onClose} />
      <div role="dialog" aria-modal="true" aria-labelledby="device-registration-title"
        className={`relative w-full max-w-lg rounded-2xl border shadow-2xl ${card}`}>
        <div className="flex items-center justify-between border-b p-6">
          <h2 id="device-registration-title" className="text-xl font-semibold">设备自动登记</h2>
          <button onClick={onClose} aria-label="关闭设备登记状态" className="rounded-lg p-2"><X className="h-5 w-5" /></button>
        </div>
        <div className="space-y-5 p-6 text-center">
          {deviceId ? (
            <>
              <CheckCircle className="mx-auto h-12 w-12 text-green-500" />
              <p role="status">设备已自动登记</p>
              <div className={secondary}>{status?.device_name || "本机设备"}</div>
              <div className={secondary}>{deviceCodeLabel(deviceId)}</div>
              <div className="font-mono text-2xl tracking-widest">{formatDeviceCode(deviceId)}</div>
              <p className={`text-sm ${secondary}`}>此设备码与本机身份绑定，下次启动会自动恢复。</p>
              <button onClick={onClose} className="w-full rounded-lg bg-blue-600 py-3 text-white">完成</button>
            </>
          ) : (
            <>
              {inProgress
                ? <Loader2 role="progressbar" aria-label={starting ? "启动后台服务" : "设备登记进度"} className="mx-auto h-12 w-12 animate-spin text-blue-500" />
                : <AlertCircle className="mx-auto h-12 w-12 text-amber-500" aria-hidden="true" />}
              <p role="status" aria-live="polite">{message}</p>
              <p className={`text-sm ${secondary}`}>{localUnavailable
                ? "后台服务运行后会自动领取设备码。如果系统弹出授权提示，请完成授权。"
                : "首次启动将自动领取设备码，无需填写登记码或设备凭据。"}</p>
              {localUnavailable && <button onClick={() => void startService()} disabled={checking || starting}
                className="w-full rounded-lg bg-blue-600 py-3 text-white disabled:opacity-50">
                {starting ? "正在启动…" : "启动后台服务"}
              </button>}
              <button onClick={() => void retry()} disabled={checking || retrying || starting}
                className="flex w-full items-center justify-center gap-2 rounded-lg bg-blue-600 py-3 text-white disabled:opacity-50">
                <RefreshCw className={`h-4 w-4 ${retrying ? "animate-spin" : ""}`} />
                {checking || retrying ? "正在检查…" : "立即重试"}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
