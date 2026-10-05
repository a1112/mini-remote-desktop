import { useCallback, useEffect, useRef, useState } from "react";
import { ChevronDown, Loader2, RefreshCw, Wifi, WifiOff } from "lucide-react";
import { useTheme } from "./ThemeContext";
import { ipcPublicServerStatus } from "../adapters/tauri/commands";
import type { PublicServerStatus } from "../adapters/tauri/types";
import { formatDeviceCode } from "../utils/deviceCode";

function connectionLabel(status: PublicServerStatus): string {
  if (!status.service_running) return "本机后台服务未运行";
  if (!status.api_url) return "公网服务器未配置";
  if (!status.device_registered) return "等待设备登记";
  switch (status.signaling_state) {
    case "authenticated": return "公网服务器已连接";
    case "connecting": return "正在连接公网服务器";
    case "backoff": return "公网连接已断开，正在重连";
    case "stopped": return "公网连接已停止";
    default: return "公网连接未启用";
  }
}

function signalingLabel(state: PublicServerStatus["signaling_state"]): string {
  switch (state) {
    case "authenticated": return "信令已认证在线";
    case "connecting": return "正在连接并验证设备身份";
    case "backoff": return "信令已断开，正在自动重连";
    case "stopped": return "信令连接已停止";
    default: return "信令连接未启用";
  }
}

/** Display only the server host. Paths, query strings and URL credentials are private. */
function serverHost(url: string | null): string {
  if (!url) return "未配置";
  try {
    const parsed = new URL(url);
    return ["https:", "http:"].includes(parsed.protocol) ? parsed.host : "地址不可用";
  } catch { return "地址不可用"; }
}

/** Map known sanitized codes; never render arbitrary transport errors or credentials. */
function connectionError(code: string | null): string | null {
  if (!code) return null;
  const known: Record<string, string> = {
    public_api_unreachable: "无法连接公网服务器，请检查网络后重试。",
    public_configuration_unavailable: "服务器接口可达，连接配置暂未就绪，请稍后重试。",
    public_credential_refresh_failed: "设备凭据刷新失败，正在重试；失效时请使用管理员恢复凭据。",
    public_credential_save_failed: "无法安全保存设备凭据，正在重试。",
    public_runtime_config_conflict: "后台存在独立公网配置，请先统一设备登记配置。",
    public_runtime_start_failed: "公网会话启动失败，请检查服务器登记配置。",
    signaling_credentials_unavailable: "设备凭据暂时不可用，请重新登记设备。",
    signaling_credentials_invalid: "设备身份验证失败，请更新设备凭据。",
    signaling_credentials_timeout: "设备身份验证超时，正在重试。",
    signaling_server_identity_mismatch: "服务器身份验证失败，请检查服务器配置。",
    signaling_role_mismatch: "设备连接权限不匹配，请检查设备登记信息。",
    signaling_transport: "无法连接信令服务器，请检查网络。",
    signaling_connect_timeout: "连接服务器超时，正在重试。",
    signaling_handshake_timeout: "服务器身份验证超时，正在重试。",
    signaling_disconnected: "服务器连接已断开，正在重试。",
    signaling_config: "服务器连接配置不完整，请检查配置。",
  };
  return known[code] ?? "服务器连接暂时不可用，请检查网络或设备登记状态。";
}

export function ServiceStatusPanel() {
  const { isDark } = useTheme();
  const [status, setStatus] = useState<PublicServerStatus | null>(null);
  const [checking, setChecking] = useState(true);
  const [failed, setFailed] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const inFlight = useRef(false);
  const mounted = useRef(false);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    if (mounted.current) setChecking(true);
    try {
      const result = await ipcPublicServerStatus();
      if (!mounted.current) return;
      setStatus(result.ok ? result.value : null);
      setFailed(!result.ok);
    } catch {
      if (mounted.current) { setStatus(null); setFailed(true); }
    } finally {
      inFlight.current = false;
      if (mounted.current) setChecking(false);
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    const interval = window.setInterval(() => { if (!document.hidden) void refresh(); }, 3000);
    const onVisible = () => { if (!document.hidden) void refresh(); };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      mounted.current = false;
      window.clearInterval(interval);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [refresh]);

  const online = Boolean(status?.service_running && status.device_registered && status.signaling_state === "authenticated");
  const label = status ? connectionLabel(status) : failed ? "无法读取连接状态" : "正在检查服务器连接";
  const error = connectionError(status?.last_error ?? null);
  const muted = isDark ? "text-gray-400" : "text-gray-500";
  const tone = online ? "text-emerald-600" : status?.signaling_state === "connecting" ? "text-blue-500" : "text-amber-600";

  return <section aria-label="服务器连接状态" className={`shrink-0 border-b px-3 py-2 ${isDark ? "bg-[#1f1f1f] border-gray-700 text-gray-200" : "bg-white border-gray-200 text-gray-800"}`}>
    <div className="flex flex-wrap items-center gap-2">
      <div className={`flex min-w-0 flex-1 items-center gap-2 text-sm ${tone}`} role="status" aria-live="polite">
        {checking && !status ? <Loader2 className="h-4 w-4 shrink-0 animate-spin" /> : online ? <Wifi className="h-4 w-4 shrink-0" /> : <WifiOff className="h-4 w-4 shrink-0" />}
        <span>{label}</span>
      </div>
      <button type="button" onClick={() => setExpanded(!expanded)} aria-expanded={expanded} className={`flex items-center gap-1 rounded px-2 py-1 text-xs ${muted}`}>
        连接详情 <ChevronDown className={`h-3.5 w-3.5 transition-transform ${expanded ? "rotate-180" : ""}`} />
      </button>
      <button type="button" aria-label="刷新连接状态" title="刷新连接状态" onClick={() => void refresh()} disabled={checking} className={`rounded p-1.5 disabled:opacity-40 ${muted}`}>
        <RefreshCw className={`h-4 w-4 ${checking ? "animate-spin" : ""}`} />
      </button>
    </div>
    {expanded && <div className={`mt-2 space-y-1.5 border-t pt-2 text-xs ${isDark ? "border-gray-700" : "border-gray-100"}`}>
      <dl className="grid grid-cols-[5rem_1fr] gap-x-2 gap-y-1.5">
        <dt className={muted}>本机后台</dt><dd>{status?.service_running ? "后台服务运行中" : status ? "后台服务未运行" : "状态未知"}</dd>
        <dt className={muted}>公网服务器</dt><dd>{serverHost(status?.api_url ?? null)}</dd>
        <dt className={muted}>服务器接口</dt><dd>{status?.api_reachable === true ? "服务器接口可达" : status?.api_reachable === false ? "服务器接口暂时不可达" : "尚未检测"}</dd>
        <dt className={muted}>设备登记</dt><dd>{status?.device_registered ? `已登记 · ${formatDeviceCode(status.device_id)}` : "设备尚未登记到服务器"}</dd>
        <dt className={muted}>信令连接</dt><dd>{status ? signalingLabel(status.signaling_state) : "状态未知"}{status && status.reconnect_attempt > 0 ? `（重试 ${status.reconnect_attempt} 次）` : ""}</dd>
        {status?.last_connected_at_ms && <><dt className={muted}>最近连接</dt><dd>{new Date(status.last_connected_at_ms).toLocaleString("zh-CN")}</dd></>}
      </dl>
      {error && <p className="text-amber-600">{error}</p>}
      {failed && <p className="text-amber-600">请确认本机后台服务已启动，然后刷新连接状态。</p>}
    </div>}
  </section>;
}
