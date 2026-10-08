import { useState } from "react";
import { Link, NavLink, useLocation, useNavigate } from "react-router";
import {
  ArrowLeft, ArrowRight, Check, ChevronRight, Clock3, Copy, FileClock,
  Home, Laptop2, LoaderCircle, Monitor, RefreshCw, Search, Settings2,
  UserRound, Wifi, WifiOff,
} from "lucide-react";
import { useTheme } from "./ThemeContext";
import { useAuth } from "./AuthContext";
import { useDevices, type Device } from "./deviceData";
import { useDeviceRegistration } from "../services/deviceService";
import { useConnectionHistory } from "../services/connectionHistoryService";
import { launchRemoteDisplayForDevice } from "../services/remoteDisplayLauncher";
import { ServiceStatusPanel } from "./ServiceStatusPanel";
import { DEVICE_CODE_INPUT_ERROR, deviceCodeLabel, formatDeviceCode, normalizeDeviceCode, parseRemoteDeviceInput } from "../utils/deviceCode";

type MobileLayoutProps = { onOpenAuth: () => void };

const tabs = [
  { to: "/", label: "首页", icon: Home, end: true },
  { to: "/devices", label: "设备", icon: Monitor, end: false },
  { to: "/connections", label: "记录", icon: FileClock, end: false },
  { to: "/settings", label: "设置", icon: Settings2, end: false },
];

function MobileHomePage() {
  const navigate = useNavigate();
  const { devices } = useDevices();
  const { deviceId: myDeviceId, deviceName: myDeviceName } = useDeviceRegistration();
  const history = useConnectionHistory();
  const [targetId, setTargetId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const recent = history.filter((entry, index, entries) =>
    entries.findIndex((candidate) => candidate.peerDeviceId === entry.peerDeviceId) === index,
  ).slice(0, 3);

  const connect = async (rawId: string) => {
    if (!rawId.trim() || busy) return;
    const parsed = parseRemoteDeviceInput(rawId);
    if (!parsed) { setError(DEVICE_CODE_INPUT_ERROR); return; }
    const deviceId = parsed.deviceId;
    const device = devices.find((item) => item.deviceId.replace(/\s/g, "") === deviceId);
    setBusy(true);
    setError(null);
    try {
      const result = await launchRemoteDisplayForDevice(deviceId, {
        transportKind: "webrtc",
        targetDeviceName: device?.name ?? "远程设备",
        targetOs: device?.os ?? "Unknown",
        routePreference: "auto",
      });
      navigate(`/session/${result.sessionId}`);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "连接请求失败");
    } finally {
      setBusy(false);
    }
  };

  const copyMyId = async () => {
    if (!myDeviceId) return;
    try {
      await navigator.clipboard.writeText(normalizeDeviceCode(myDeviceId));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      setError("复制设备 ID 失败");
    }
  };

  return <div className="mobile-page">
    <div className="mobile-intro">
      <p className="mobile-eyebrow">安全远程访问</p>
      <h1>连接远程设备</h1>
      <p>输入 9 位远程设备码，发起连接请求</p>
    </div>

    <section className="mobile-card mobile-connect-card" aria-label="连接远程设备">
      <label htmlFor="mobile-target-id" className="mobile-field-label">远程设备码</label>
      <div className="mobile-input-wrap">
        <Monitor size={20} aria-hidden="true" />
        <input id="mobile-target-id" inputMode="text" autoComplete="off" placeholder="输入 9 位设备码"
          value={targetId} onChange={(event) => setTargetId(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter") void connect(targetId); }} />
      </div>
      <p className="mobile-state-text">已有的旧设备码和局域网标识仍可使用。</p>
      <button className="mobile-primary-button" disabled={!targetId.trim() || busy} onClick={() => void connect(targetId)}>
        {busy ? <LoaderCircle size={19} className="animate-spin" /> : null}
        {busy ? "连接中…" : "发起连接"}
        {!busy ? <ArrowRight size={20} aria-hidden="true" /> : null}
      </button>
      {error ? <p className="mobile-error" role="alert">{error}</p> : null}
    </section>

    <section className="mobile-card mobile-recent-card" aria-labelledby="mobile-recent-heading">
      <div className="mobile-section-heading">
        <h2 id="mobile-recent-heading"><Clock3 size={20} />最近连接</h2>
        <Link to="/connections">查看更多 <ChevronRight size={15} /></Link>
      </div>
      {recent.length === 0 ? <div className="mobile-empty mobile-recent-empty">
        <FileClock size={46} strokeWidth={1.35} aria-hidden="true" />
        <strong>暂无连接记录</strong>
        <span>连接过的设备将显示在这里</span>
      </div> : <div className="mobile-recent-list">{recent.map((entry) => {
        const device = devices.find((item) => item.deviceId === entry.peerDeviceId);
        return <button key={entry.sessionId} className="mobile-recent-item" onClick={() => void connect(entry.peerDeviceId)} disabled={busy}>
          <span className="mobile-device-icon"><Monitor size={20} /></span>
          <span className="mobile-recent-copy"><strong>{device?.name ?? entry.peerDeviceId}</strong><small>{new Date(entry.startedAt).toLocaleString("zh-CN")}</small></span>
          <ChevronRight size={18} aria-hidden="true" />
        </button>;
      })}</div>}
    </section>

    <section className="mobile-local-device" aria-label="本机设备">
      <span><small>{deviceCodeLabel(myDeviceId)}</small><strong>{myDeviceName || "设备码未就绪"}</strong><code>{myDeviceId ? formatDeviceCode(myDeviceId) : "等待设备登记"}</code></span>
      <button onClick={() => void copyMyId()} disabled={!myDeviceId} aria-label={copied ? "已复制设备 ID" : "复制本机设备 ID"}>
        {copied ? <Check size={19} /> : <Copy size={19} />}
      </button>
    </section>
  </div>;
}

function MobileDevicesPage({ onOpenAuth }: MobileLayoutProps) {
  const { devices, loading, error, refresh } = useDevices({ pollInterval: 30000, enabled: true });
  const { isLoggedIn } = useAuth();
  const [query, setQuery] = useState("");
  const [refreshing, setRefreshing] = useState(false);
  const remoteDevices = devices.filter((device) => !device.isLocal);
  const filtered = remoteDevices.filter((device) =>
    `${device.name} ${device.deviceId}`.toLowerCase().includes(query.trim().toLowerCase()),
  );
  const doRefresh = async () => {
    setRefreshing(true);
    try { await refresh(); } finally { setRefreshing(false); }
  };
  return <div className="mobile-page">
    <div className="mobile-intro mobile-intro-with-action">
      <div><p className="mobile-eyebrow">设备管理</p><h1>我的设备</h1><p>查看已发现和已同步的设备</p></div>
      <button className="mobile-icon-button" aria-label="刷新设备" onClick={() => void doRefresh()} disabled={refreshing}>
        <RefreshCw size={20} className={refreshing ? "animate-spin" : ""} />
      </button>
    </div>
    <label className="mobile-search"><Search size={20} aria-hidden="true" />
      <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索设备名称或 ID" aria-label="搜索设备" />
    </label>
    {error ? <p className="mobile-error" role="alert">{error}</p> : null}
    {loading && remoteDevices.length === 0 ? <p className="mobile-state-text">正在读取设备…</p> : null}
    {!loading && remoteDevices.length === 0 ? <div className="mobile-card mobile-devices-empty mobile-empty">
      <Laptop2 size={64} strokeWidth={1.3} aria-hidden="true" />
      <strong>暂无已发现设备</strong>
      <span>{isLoggedIn ? "刷新后查看已同步或局域网发现的设备" : "登录后可同步设备，也可连接已知设备 ID"}</span>
      <button className="mobile-primary-button mobile-empty-button" onClick={isLoggedIn ? () => void doRefresh() : onOpenAuth}>
        {isLoggedIn ? "刷新设备" : "登录后同步设备"}
      </button>
    </div> : null}
    {!loading && remoteDevices.length > 0 && filtered.length === 0 ? <p className="mobile-state-text">没有符合条件的设备</p> : null}
    <div className="mobile-device-list">{filtered.map((device) => <Link to={`/devices/${device.id}`} className="mobile-card mobile-device-row" key={device.id}>
      <span className="mobile-device-icon"><Monitor size={24} /></span>
      <span className="mobile-device-copy"><strong>{device.name}</strong><small>{device.deviceId}</small><em>{device.sourceLabel}</em></span>
      <span className={`mobile-device-status ${device.status === "online" ? "online" : ""}`}>
        {device.status === "online" ? <Wifi size={15} /> : <WifiOff size={15} />}{device.status === "online" ? "在线" : "离线"}
      </span>
      <ChevronRight size={17} className="mobile-row-chevron" />
    </Link>)}</div>
  </div>;
}

function MobileDeviceDetailPage({ deviceId }: { deviceId: string }) {
  const navigate = useNavigate();
  const { devices, loading } = useDevices({ pollInterval: 30000, enabled: true });
  const device = devices.find((item) => item.id === deviceId || item.deviceId === deviceId);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const connect = async (target: Device) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const result = await launchRemoteDisplayForDevice(target.deviceId, {
        transportKind: target.p2pAvailable ? "quic" : "webrtc",
        targetDeviceName: target.name,
        targetOs: target.os,
        targetIp: target.ip,
        lanP2P: target.p2pAvailable && !target.isLocal,
      });
      if (result.mode === "route") navigate(`/session/${result.sessionId}`);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "连接请求失败");
    } finally { setBusy(false); }
  };
  return <div className="mobile-page">
    <button className="mobile-back" onClick={() => navigate("/devices")}><ArrowLeft size={19} />返回设备</button>
    {loading && !device ? <p className="mobile-state-text">正在读取设备…</p> : null}
    {!loading && !device ? <p className="mobile-state-text">设备未找到</p> : null}
    {device ? <>
      <div className="mobile-intro"><p className="mobile-eyebrow">设备详情</p><h1>{device.name}</h1><p>{device.deviceId}</p></div>
      <div className="mobile-card mobile-detail-card">
        <span className="mobile-device-icon"><Monitor size={34} /></span>
        <div><strong>{device.status === "online" ? "设备在线" : "设备离线"}</strong><p>{device.sourceLabel}</p><p>{device.os}</p></div>
      </div>
      <button className="mobile-primary-button" disabled={device.status !== "online" || device.disabled || device.isLocal || busy} onClick={() => void connect(device)}>
        {busy ? "连接中…" : "发起远程连接"}<ArrowRight size={20} />
      </button>
      {error ? <p className="mobile-error" role="alert">{error}</p> : null}
    </> : null}
  </div>;
}

function MobileHistoryPage() {
  const history = useConnectionHistory();
  const { devices } = useDevices();
  return <div className="mobile-page">
    <div className="mobile-intro"><p className="mobile-eyebrow">本机记录</p><h1>连接记录</h1><p>远程画面开始传输后才会记录</p></div>
    {history.length === 0 ? <div className="mobile-card mobile-history-empty mobile-empty">
      <FileClock size={60} strokeWidth={1.3} aria-hidden="true" />
      <strong>暂无连接记录</strong><span>连接过的设备将显示在这里</span>
    </div> : <div className="mobile-history-list">{history.map((entry) => {
      const device = devices.find((item) => item.deviceId === entry.peerDeviceId);
      return <article className="mobile-card mobile-history-row" key={entry.sessionId}>
        <span className="mobile-device-icon"><Monitor size={21} /></span>
        <div><strong>{device?.name ?? entry.peerDeviceId}</strong><small>{entry.peerDeviceId}</small><small>{new Date(entry.startedAt).toLocaleString("zh-CN")}</small></div>
        <em>{entry.role === "controller" ? "主动连接" : "被动接入"}</em>
      </article>;
    })}</div>}
  </div>;
}

function MobileSettingsPage({ onOpenAuth }: MobileLayoutProps) {
  const { theme, setTheme } = useTheme();
  const { isLoggedIn, user, logout } = useAuth();
  const { deviceId } = useDeviceRegistration();
  return <div className="mobile-page">
    <div className="mobile-intro"><p className="mobile-eyebrow">个人偏好</p><h1>设置</h1><p>管理显示方式与账户</p></div>
    <section className="mobile-card mobile-settings-card">
      <h2>外观</h2>
      <div className="mobile-theme-options" role="group" aria-label="界面主题">
        {(["dark", "light", "system"] as const).map((option) => <button key={option} className={theme === option ? "selected" : ""} onClick={() => setTheme(option)}>
          {{ dark: "深色", light: "浅色", system: "跟随系统" }[option]}
        </button>)}
      </div>
    </section>
    <section className="mobile-card mobile-settings-card">
      <h2>账户</h2>
      <div className="mobile-account-row"><UserRound size={22} /><span>{isLoggedIn ? user?.username : "尚未登录"}</span></div>
      <button className="mobile-secondary-button" onClick={isLoggedIn ? logout : onOpenAuth}>{isLoggedIn ? "退出登录" : "登录账户"}</button>
    </section>
    <section className="mobile-card mobile-settings-card">
      <h2>本机设备码</h2><code>{deviceId ? formatDeviceCode(deviceId) : "等待设备登记"}</code>
    </section>
  </div>;
}

export function MobileLayout({ onOpenAuth }: MobileLayoutProps) {
  const location = useLocation();
  const navigate = useNavigate();
  const { isDark } = useTheme();
  const deviceDetailId = location.pathname.startsWith("/devices/") ? decodeURIComponent(location.pathname.slice("/devices/".length)) : null;
  return <div className="mobile-shell" data-theme={isDark ? "dark" : "light"}>
    <header className="mobile-header">
      <Link to="/" className="mobile-brand" aria-label="R-Desk 首页"><span className="mobile-mark">R</span><span>R-Desk</span></Link>
      <button className="mobile-header-action" aria-label="打开设置" onClick={() => navigate("/settings")}><Settings2 size={23} /></button>
    </header>
    <ServiceStatusPanel />
    <main className="mobile-main" id="mobile-main">
      {location.pathname === "/" ? <MobileHomePage /> : null}
      {location.pathname === "/devices" ? <MobileDevicesPage onOpenAuth={onOpenAuth} /> : null}
      {deviceDetailId ? <MobileDeviceDetailPage deviceId={deviceDetailId} /> : null}
      {location.pathname === "/connections" ? <MobileHistoryPage /> : null}
      {location.pathname === "/settings" ? <MobileSettingsPage onOpenAuth={onOpenAuth} /> : null}
      {!(["/", "/devices", "/connections", "/settings"].includes(location.pathname) || deviceDetailId) ? <MobileHomePage /> : null}
    </main>
    <nav className="mobile-tabs" aria-label="移动端导航">{tabs.map(({ to, label, icon: Icon, end }) =>
      <NavLink key={to} to={to} end={end} className={({ isActive }) => `mobile-tab ${isActive ? "active" : ""}`}>
        <Icon size={23} strokeWidth={1.8} aria-hidden="true" /><span>{label}</span>
      </NavLink>,
    )}</nav>
  </div>;
}
