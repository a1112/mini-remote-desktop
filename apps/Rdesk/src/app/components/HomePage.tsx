import { useState, useEffect, useId, useRef } from "react";
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
  ArrowDownLeft,
  ArrowUpRight,
  Globe,
  ShieldCheck,
} from "lucide-react";
import { useAuth } from "./AuthContext";
import { useDeviceRegistration, deviceService } from "../services/deviceService";
import { launchRemoteDisplayForDevice } from "../services/remoteDisplayLauncher";
import { closeBrowserRemoteSession } from "../services/browserRemoteSessionService";
import { useDevices } from "./deviceData";
import { useConnectionHistory } from "../services/connectionHistoryService";
import { DeviceRegisterModal } from "./DeviceRegisterModal";
import { DEVICE_CODE_INPUT_ERROR, formatDeviceCode, normalizeDeviceCode, parseRemoteDeviceInput } from "../utils/deviceCode";
import { isTauriRuntime } from '../utils/runtime';
import { GuestRemoteConnectForm } from './GuestRemoteConnectForm';
import { TemporaryAccessPasswordCard } from './TemporaryAccessPasswordCard';

export function HomePage() {
  const native = isTauriRuntime();
  const { isLoggedIn, token } = useAuth();
  const navigate = useNavigate();
  const deviceCodeInputId = useId();
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
        lastConnected: new Date(entry.startedAt),
      };
    });
  const { deviceId: myDeviceId, deviceName: myDeviceName, registrationError, refresh, reregister, serviceManagedRegistration } = useDeviceRegistration();
  const [registrationOpen, setRegistrationOpen] = useState(false);
  const [connectId, setConnectId] = useState("");
  const [copyStatus, setCopyStatus] = useState<"copied" | "failed" | null>(null);
  const [guestTarget, setGuestTarget] = useState<{ deviceId: string } | null>(null);
  const [guestBusy, setGuestBusy] = useState(false);
  const [connecting, setConnecting] = useState<{ id: string; source: "form" | "recent" } | null>(null);
  const [connectionError, setConnectionError] = useState<{ message: string; source: "form" | "recent" } | null>(null);
  const [useTemporaryPassword, setUseTemporaryPassword] = useState(false);
  const connectionInFlightRef = useRef(false);
  const browserConnection = useRef<AbortController | null>(null);
  const showDeviceCodeForm = native || (isLoggedIn && !useTemporaryPassword);

  useEffect(() => () => browserConnection.current?.abort(), [token]);

  const [editingDeviceName, setEditingDeviceName] = useState(false);
  const [deviceNameEdit, setDeviceNameEdit] = useState("");
  const [savedName, setSavedName] = useState<{ id: string; name: string } | null>(null);
  const [renameError, setRenameError] = useState(false);
  const [savingName, setSavingName] = useState(false);
  const deviceInputRef = useRef<HTMLInputElement>(null);
  const renameInFlight = useRef(false);
  const displayDeviceName = savedName && savedName.id === myDeviceId ? savedName.name : myDeviceName;

  useEffect(() => {
    if (!copyStatus) return;
    const timer = window.setTimeout(() => setCopyStatus(null), 2200);
    return () => window.clearTimeout(timer);
  }, [copyStatus]);

  const handleCopy = async () => {
    if (!myDeviceId) return;
    try {
      await navigator.clipboard.writeText(normalizeDeviceCode(myDeviceId));
      setCopyStatus("copied");
    } catch {
      setCopyStatus("failed");
    }
  };

  const handleStartEditDeviceName = () => {
    setRenameError(false);
    setDeviceNameEdit(displayDeviceName ?? "");
    setEditingDeviceName(true);
  };

  const handleConfirmEditDeviceName = async () => {
    const name = deviceNameEdit.trim();
    if (!name || !myDeviceId || renameInFlight.current) return;
    renameInFlight.current = true;
    setSavingName(true);
    setRenameError(false);
    try {
      if (await deviceService.renameDevice(myDeviceId, name)) {
        setSavedName({ id: myDeviceId, name });
        setEditingDeviceName(false);
      } else {
        setRenameError(true);
      }
    } catch {
      setRenameError(true);
    } finally {
      setSavingName(false);
      renameInFlight.current = false;
    }
  };

  const myId = formatDeviceCode(myDeviceId);
  const hasTenDigitCode = /^\d{10}$/.test(normalizeDeviceCode(myDeviceId ?? ""));

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
  }, source: "form" | "recent") => {
    if (connectionInFlightRef.current) return;
    if (native && myDeviceId && normalizeDeviceCode(target.deviceId) === normalizeDeviceCode(myDeviceId)) {
      setConnectionError({ message: "这是本机设备码。请输入要控制的设备码。", source });
      return;
    }
    connectionInFlightRef.current = true;
    const cancellation = native ? null : new AbortController();
    browserConnection.current = cancellation;
    const targetDeviceId = target.deviceId.replace(/\s/g, "");
    setConnecting({ id: target.id, source });
    setConnectionError(null);
    try {
      const result = await launchRemoteDisplayForDevice(targetDeviceId, {
        transportKind: "webrtc",
        targetDeviceName: target.name,
        targetOs: target.os,
        routePreference: "auto",
        ...(cancellation ? { signal: cancellation.signal } : {}),
      });
      if (cancellation?.signal.aborted) {
        await closeBrowserRemoteSession(result.sessionId, 'connection_form_left');
        return;
      }
      browserConnection.current = null;
      navigate(result.routePath ?? `/session/${result.sessionId}`);
    } catch (error) {
      if (!cancellation?.signal.aborted) {
        setConnectionError({ message: error instanceof Error ? error.message : "无法发起连接，请重试。", source });
      }
    } finally {
      browserConnection.current = null;
      connectionInFlightRef.current = false;
      setConnecting(null);
    }
  };

  const handleConnect = () => {
    if (!connectId.trim()) return;
    const parsed = parseRemoteDeviceInput(connectId);
    if (!parsed) {
      setConnectionError({ message: DEVICE_CODE_INPUT_ERROR, source: "form" });
      return;
    }
    const found = devices.find((device) => normalizeDeviceCode(device.deviceId) === parsed.deviceId);
    if (found) {
      void launchSecureRemote(found, "form");
    } else {
      void launchSecureRemote({
        id: "custom",
        name: "远程设备",
        deviceId: parsed.deviceId,
        os: "Unknown",
      }, "form");
    }
  };

  const reconnect = (device: (typeof recentConnections)[number]) => {
    if (showDeviceCodeForm) {
      void launchSecureRemote(device, "recent");
    } else {
      setGuestTarget({ deviceId: device.deviceId });
    }
  };

  const connectionPending = connecting !== null || guestBusy;
  const errorMessage = (source: "form" | "recent") => connectionError?.source === source && (
    <p className="remote-home-error" role="alert">
      <AlertCircle size={16} aria-hidden="true" />
      <span>{connectionError.message}</span>
    </p>
  );

  return (
    <div className="remote-home">
      {native && <DeviceRegisterModal
        isOpen={registrationOpen}
        serviceManaged={serviceManagedRegistration}
        onClose={() => setRegistrationOpen(false)}
        onSuccess={refresh}
      />}
      <header className="remote-page-header">
        <h1>远程控制</h1>
        <p>{native ? "连接另一台设备，或将本机设备码分享给需要协助你的人。" : "输入设备码，发起远程控制。"}</p>
      </header>

      <div className="remote-home-grid">
        <section className="remote-home-card remote-home-connect" aria-label="连接远程设备">
          <div className="remote-home-card-heading">
            <span className="remote-home-icon remote-home-icon-accent"><ArrowUpRight size={21} aria-hidden="true" /></span>
            <div>
              <h2>连接远程设备</h2>
              <p>{native ? "输入对方的设备码以发起连接" : "通过浏览器控制对方的设备"}</p>
            </div>
          </div>

          {!native && isLoggedIn && <button type="button" disabled={connectionPending}
            onClick={() => { setUseTemporaryPassword(value => !value); setGuestTarget(null); setConnectionError(null); }}
            className="remote-home-link">
            {useTemporaryPassword ? "使用当前账号连接" : "使用远端临时密码"}
          </button>}

          {showDeviceCodeForm ? (
            <form className="remote-home-form" onSubmit={(event) => { event.preventDefault(); handleConnect(); }}>
              <label htmlFor={deviceCodeInputId}>远程设备码</label>
              <input
                id={deviceCodeInputId}
                className="remote-home-input"
                type="text"
                autoComplete="off"
                spellCheck={false}
                value={connectId}
                onChange={(event) => {
                  setConnectId(event.target.value);
                  if (connectionError?.source === "form") setConnectionError(null);
                }}
                aria-invalid={connectionError?.source === "form" || undefined}
                aria-describedby={`${deviceCodeInputId}-hint`}
                placeholder="输入 10 位设备码"
                disabled={connectionPending}
              />
              <p id={`${deviceCodeInputId}-hint`} className="remote-home-hint">{native
                ? "也支持旧设备码和局域网标识。"
                : "也支持已有的 9 位设备码。"}</p>
              <button type="submit" className="remote-home-primary" disabled={!connectId.trim() || connectionPending}>
                {connecting?.source === "form" ? (
                  <><Loader2 size={16} className="animate-spin" aria-hidden="true" /> 正在发起连接</>
                ) : (
                  <>立即连接 <ArrowRight size={16} aria-hidden="true" /></>
                )}
              </button>
              {errorMessage("form")}
              {native && <p className="remote-home-policy">
                <ShieldCheck size={16} aria-hidden="true" />
                <span><strong>目标设备授权策略</strong>决定此次连接允许的权限。</span>
              </p>}
            </form>
          ) : <GuestRemoteConnectForm target={guestTarget} onBusyChange={setGuestBusy} />}
        </section>

        {native ? (
          <section className="remote-home-card remote-home-local" aria-label="允许远程访问本机">
            <div className="remote-home-card-heading">
              <span className="remote-home-icon"><ArrowDownLeft size={21} aria-hidden="true" /></span>
              <div>
                <h2>允许远程访问本机</h2>
                {!editingDeviceName && <div className="remote-home-device-name">
                  <p title={displayDeviceName ?? undefined}>{displayDeviceName || "本机设备"}</p>
                  {isLoggedIn && myDeviceId && <button type="button" className="remote-home-icon-button" onClick={handleStartEditDeviceName} aria-label="编辑设备名称">
                    <Pencil size={13} aria-hidden="true" />
                  </button>}
                </div>}
              </div>
            </div>
            {editingDeviceName && <form className="remote-home-rename" onSubmit={(event) => { event.preventDefault(); void handleConfirmEditDeviceName(); }}>
              <input ref={deviceInputRef} className="remote-home-input" aria-label="设备名称" value={deviceNameEdit}
                onChange={(event) => { setDeviceNameEdit(event.target.value); setRenameError(false); }}
                onKeyDown={(event) => {
                  if (event.key === "Escape" && !savingName) setEditingDeviceName(false);
                }} disabled={savingName} />
              <button type="submit" className="remote-home-icon-button" aria-label="保存设备名称" disabled={!deviceNameEdit.trim() || savingName}>
                <Check size={16} aria-hidden="true" />
              </button>
              <button type="button" className="remote-home-icon-button" aria-label="取消编辑设备名称" onClick={() => setEditingDeviceName(false)} disabled={savingName}>
                <X size={16} aria-hidden="true" />
              </button>
            </form>}
            {editingDeviceName && renameError && <p className="remote-home-error" role="alert">无法更新设备名称，请检查设备绑定或稍后重试。</p>}
            <div className="remote-home-device-code">
              <div>
                <span className="remote-home-hint">{hasTenDigitCode ? "10 位设备码" : "当前设备标识"}</span>
                <p className={myDeviceId ? "remote-home-code-value" : "remote-home-code-pending"}>{myId}</p>
              </div>
              <button type="button" className="remote-home-icon-button" aria-label={copyStatus === "copied" ? "已复制" : "复制设备码"}
                title={copyStatus === "copied" ? "已复制" : "复制设备码"} onClick={() => void handleCopy()} disabled={!myDeviceId}>
                {copyStatus === "copied" ? <Check size={17} aria-hidden="true" /> : <Copy size={17} aria-hidden="true" />}
              </button>
            </div>
            {copyStatus === "copied" && <span className="sr-only" role="status">已复制设备码</span>}
            {copyStatus === "failed" && <p className="remote-home-error" role="alert">无法写入剪贴板，请手动复制设备码。</p>}
            <TemporaryAccessPasswordCard />
            {registrationError && <p role="alert" className="remote-home-error">{registrationError}</p>}
            <div className="remote-home-registration">
              {serviceManagedRegistration && <button type="button" onClick={() => { void reregister(); }} className="remote-home-link">刷新登记状态</button>}
              <button type="button" onClick={() => setRegistrationOpen(true)} className="remote-home-link">
                {serviceManagedRegistration ? "管理员登记或恢复" : "登记到服务器"}
              </button>
            </div>
          </section>
        ) : (
          <section className="remote-home-card remote-home-web" aria-label="网页控制端">
            <div className="remote-home-card-heading">
              <span className="remote-home-icon"><Globe size={21} aria-hidden="true" /></span>
              <div><h2>网页控制端</h2><p>当前浏览器可发起远程连接</p></div>
            </div>
            <p>无需安装客户端或登记本机。{isLoggedIn
              ? "使用当前账号连接你可访问的设备，或改用对方提供的临时密码。"
              : "请向对方索取设备码和临时密码，或登录账号访问你的设备。"}</p>
          </section>
        )}
      </div>

      <section className="remote-home-card remote-home-history" aria-label="最近连接">
        <div className="remote-home-history-heading">
          <h2><Clock size={19} aria-hidden="true" /> 最近连接</h2>
          <button type="button" className="remote-home-link" onClick={() => navigate("/connections")}>
            查看全部 <ChevronRight size={15} aria-hidden="true" />
          </button>
        </div>
        {errorMessage("recent")}
        {recentConnections.length > 0 ? <ul>
          {recentConnections.map((device) => {
            const Icon = device.icon;
            return (
              <li key={device.deviceId}>
                <button type="button" className="remote-home-history-row" onClick={() => reconnect(device)} disabled={connectionPending}>
                  <span className="remote-home-icon"><Icon size={18} aria-hidden="true" /></span>
                  <span className="remote-home-history-name">
                    <strong>{device.name}</strong>
                    <small>{formatDeviceCode(device.deviceId)}{device.os ? ` · ${device.os}` : ""}</small>
                  </span>
                  {device.status && <span className="remote-home-history-status" data-online={device.status === "online"}>{device.status === "online" ? "在线" : "离线"}</span>}
                  {Number.isFinite(device.lastConnected.getTime()) && <time dateTime={device.lastConnected.toISOString()}>{device.lastConnected.toLocaleString("zh-CN")}</time>}
                  <span className="remote-home-history-action">
                    {connecting?.source === "recent" && connecting.id === device.id ? <Loader2 size={16} className="animate-spin" aria-hidden="true" /> : <ArrowRight size={16} aria-hidden="true" />}
                    <span className="sr-only">{connecting?.source === "recent" && connecting.id === device.id ? "正在发起连接" : "再次连接"}</span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul> : (
          <div className="remote-home-empty">
            <span className="remote-home-icon"><Clock size={20} aria-hidden="true" /></span>
            <div><strong>暂无连接记录</strong><p>成功建立的远程连接会显示在这里。</p></div>
          </div>
        )}
      </section>
    </div>
  );
}
