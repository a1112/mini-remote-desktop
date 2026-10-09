import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { Bell, LogOut, Monitor, Palette, Shield, User, Volume2, Wifi } from 'lucide-react';
import { useAuth } from './AuthContext';
import { useTheme } from './ThemeContext';
import { listSessions } from '../services/ipcSessionService';
import { isTauriRuntime } from '../utils/runtime';
import {
  ffmpegDownload, ffmpegProbe, ffmpegResetGoldenSettings, getDecodePolicy,
  getServiceAutostart, getServiceHealth, getUiPreferences, quitUiAndStopService,
  serviceRestart, serviceStart, serviceStop, setCloseBehavior, setServiceAutostart,
  type CloseBehavior,
} from '../services/serviceLifecycleService';

const sections = [
  { id: 'general', label: '通用', title: '通用设置', icon: Monitor },
  { id: 'security', label: '安全', title: '安全设置', icon: Shield },
  { id: 'network', label: '网络', title: '网络设置', icon: Wifi },
  { id: 'display', label: '显示', title: '显示设置', icon: Palette },
  { id: 'audio', label: '音频与输入', title: '音频与输入设置', icon: Volume2 },
  { id: 'notifications', label: '通知', title: '通知设置', icon: Bell },
  { id: 'account', label: '账户', title: '账户设置', icon: User },
] as const;

const errorMessage = (error: unknown) => error instanceof Error ? error.message : String(error);

// A request owns its loading state. Unmount/StrictMode cleanup invalidates it.
function useSettingValue<T>(load: () => Promise<T>, autoLoad = true) {
  const [value, setValue] = useState<T | null>(null);
  const [loading, setLoading] = useState(autoLoad);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const alive = useRef(false);
  const generation = useRef(0);
  const busy = useRef(false);
  const run = useCallback(async (operation: () => Promise<T>, successMessage?: string) => {
    if (busy.current || !alive.current) return;
    busy.current = true;
    const request = ++generation.current;
    setLoading(true); setError(null); setMessage(null);
    try {
      const next = await operation();
      if (alive.current && request === generation.current) {
        setValue(next); setMessage(successMessage ?? null);
      }
    } catch (failure) {
      if (alive.current && request === generation.current) setError(errorMessage(failure));
    } finally {
      if (alive.current && request === generation.current) { busy.current = false; setLoading(false); }
    }
  }, []);
  useEffect(() => {
    alive.current = true; busy.current = false;
    if (autoLoad) void run(load);
    return () => { alive.current = false; generation.current += 1; };
  }, [load, autoLoad, run]);
  return { value, loading, error, message, run, refresh: () => run(load) };
}

const noInitialAction = async () => null;

function Action({ children, onClick, disabled = false, label }: { children: ReactNode; onClick: () => void; disabled?: boolean; label?: string }) {
  return <button type="button" aria-label={label} disabled={disabled} onClick={onClick}
    className="rounded-lg border border-gray-500/30 px-3 py-1.5 text-xs transition-colors hover:bg-gray-500/10 disabled:cursor-not-allowed disabled:opacity-50">{children}</button>;
}

function Row({ label, description, children }: { label: string; description?: string; children: ReactNode }) {
  return <div className="flex flex-wrap items-center justify-between gap-3 border-b border-gray-500/10 py-3.5">
    <div className="min-w-0 flex-1"><div className="text-[13px] font-medium">{label}</div>{description && <p className="mt-1 text-xs leading-relaxed text-gray-500">{description}</p>}</div>
    {children}
  </div>;
}

function Toggle({ label, value, disabled, onChange }: { label: string; value: boolean; disabled: boolean; onChange?: (value: boolean) => void }) {
  return <button type="button" role="switch" aria-label={label} aria-checked={value} disabled={disabled}
    onClick={() => onChange?.(!value)} className={`relative h-[22px] w-10 shrink-0 rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${value ? 'bg-blue-600' : 'bg-gray-500/40'}`}>
    <span className="absolute top-0.5 h-[18px] w-[18px] rounded-full bg-white shadow-sm transition-all" style={{ left: value ? 20 : 2 }} />
  </button>;
}

function Unsupported({ label, description, toggle = false }: { label: string; description?: string; toggle?: boolean }) {
  return <Row label={label} description={description}><div className="flex items-center gap-3"><span className="text-xs text-gray-500">暂不支持</span>{toggle && <Toggle label={label} value={false} disabled />}</div></Row>;
}

function Feedback({ error, message, loading }: { error?: string | null; message?: string | null; loading?: boolean }) {
  return <>{loading && <p className="mt-2 text-xs text-gray-500" role="status">正在读取或保存…</p>}{error && <p className="mt-2 break-words text-xs text-red-500" role="alert">{error}</p>}{message && <p className="mt-2 text-xs text-green-600" role="status">{message}</p>}</>;
}

function ConnectionSummary() {
  const sessions = useSettingValue(listSessions);
  const stateLabels: Record<string, string> = { created: '已创建', listening: '等待连接', connecting: '连接中', connected: '已连接', streaming: '传输中', failed: '失败', closed: '已关闭' };
  return <div className="mt-4 rounded-xl border border-gray-500/20 p-3.5">
    <div className="flex items-center justify-between gap-3"><h4 className="text-[13px] font-medium">已有连接</h4><Action disabled={sessions.loading} onClick={() => void sessions.refresh()}>刷新连接信息</Action></div>
    <Feedback {...sessions} />
    {sessions.value?.length === 0 && <p className="mt-3 text-xs text-gray-500">暂无连接</p>}
    <ul className="mt-3 space-y-3 text-xs">{sessions.value?.map(session => <li key={session.session_id} className="min-w-0 rounded-lg bg-gray-500/5 p-3">
      <p className="break-all font-medium">{session.peer_device_id || session.session_id}</p>
      <p className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-gray-500"><span>{stateLabels[session.state] ?? session.state}</span><span>{session.transport_kind}</span></p>
      {session.last_error && <p className="mt-1 break-words text-red-500">{session.last_error}</p>}
    </li>)}</ul>
  </div>;
}

export function SettingsContent() {
  const [active, setActive] = useState<string>('general');
  const { theme, setTheme, isDark } = useTheme();
  const { user, isLoggedIn, logout, logoutError } = useAuth();
  const native = isTauriRuntime();
  const preferences = useSettingValue(getUiPreferences, native);
  const autostart = useSettingValue(getServiceAutostart, native);
  const health = useSettingValue(getServiceHealth, native);
  const decode = useSettingValue(getDecodePolicy, native);
  const ffmpeg = useSettingValue(ffmpegProbe, native);
  const quit = useSettingValue(noInitialAction, false);
  const inputStyle = `max-w-full rounded-lg border px-3 py-1.5 text-xs outline-none disabled:cursor-not-allowed disabled:opacity-50 ${isDark ? 'border-gray-600 bg-[#2a2a2a] text-gray-200' : 'border-gray-200 bg-white text-gray-700'}`;
  const section = sections.find(item => item.id === active)!;
  const runServiceAction = (operation: () => Promise<boolean>) => health.run(async () => { await operation(); return getServiceHealth(); });

  return <div className={`flex min-h-0 flex-1 flex-col overflow-hidden sm:flex-row ${isDark ? 'text-gray-200' : 'text-gray-800'}`}>
    <nav aria-label="设置分类" className="flex shrink-0 gap-1 overflow-auto border-b border-gray-500/15 p-2 sm:w-40 sm:flex-col sm:border-r sm:border-b-0 sm:p-3">
      {sections.map(({ id: key, label, icon: Icon }) => <button type="button" key={key} aria-pressed={active === key}
        onClick={() => setActive(key)} className={`flex shrink-0 items-center gap-2.5 rounded-lg px-3 py-2.5 text-left text-[13px] transition-colors ${active === key ? 'bg-blue-500/10 text-blue-500' : 'text-gray-500 hover:bg-gray-500/10'}`}>
        <Icon className="h-4 w-4 shrink-0" />{label}
      </button>)}
    </nav>
    <section aria-label={section.title} className="min-w-0 flex-1 overflow-y-auto p-4 sm:p-6">
      <h3 className="mb-2 text-sm font-semibold">{section.title}</h3>
      {!native && ['general', 'network', 'display'].includes(active) && <p className="my-3 text-xs leading-relaxed text-gray-500">网页控制端无需本机后台服务。系统关闭行为、后台服务和媒体工具设置仅在桌面客户端可用；网页画面使用浏览器 WebRTC 解码。</p>}
      {active === 'general' && <>
        <Row label="开机自动启动" description="系统启动时运行后台服务，未打开窗口也可接受连接">
          <Toggle label="后台服务开机启动" value={autostart.value?.enabled ?? false} disabled={autostart.loading || !autostart.value?.supported || quit.loading}
            onChange={enabled => void autostart.run(() => setServiceAutostart(enabled), '开机启动配置已保存')} />
        </Row>
        {autostart.value?.supported === false && <p className="mt-2 text-xs text-gray-500">当前运行方式不支持开机启动，请先安装后台服务。</p>}
        <Feedback {...autostart} />
        <Row label="关闭窗口时" description="退出界面后，后台服务仍可运行并接受连接">
          <select aria-label="关闭窗口时" className={inputStyle} value={preferences.value?.close_behavior ?? 'hide_to_tray'} disabled={preferences.loading || !preferences.value}
            onChange={event => { const behavior = event.target.value as CloseBehavior; void preferences.run(() => setCloseBehavior(behavior), '关闭行为已保存'); }}>
            <option value="hide_to_tray">最小化到托盘</option><option value="exit_ui">退出界面</option>
          </select>
        </Row>
        <Feedback {...preferences} />
        <Row label="界面主题" description="立即应用并保存到此设备">
          <select aria-label="界面主题" className={inputStyle} value={theme} onChange={event => setTheme(event.target.value as typeof theme)}>
            <option value="light">浅色</option><option value="dark">深色</option><option value="system">跟随系统</option>
          </select>
        </Row>
        <Unsupported label="界面语言" description="当前界面使用简体中文" />
        <Row label="退出并停止后台服务" description="结束当前远程连接，确认服务停止后退出窗口">
          <Action disabled={!native || quit.loading || autostart.loading || health.loading} onClick={() => void quit.run(async () => { await quitUiAndStopService(); return null; })}>{quit.loading ? '正在停止…' : '退出并停止后台服务'}</Action>
        </Row><Feedback {...quit} />
      </>}
      {active === 'security' && <>
        <p className="mb-2 text-xs leading-relaxed text-gray-500">以下配置尚未接入后台。每次远程连接仍使用现有授权流程。</p>
        <Unsupported label="双因素认证" description="账户的额外验证" toggle />
        <Unsupported label="连接密码" description="连接密码策略设置" toggle />
        <Unsupported label="空闲自动锁定" description="空闲超时断开策略" toggle />
      </>}
      {active === 'network' && <>
        <div className="mt-3 rounded-xl border border-gray-500/20 p-3.5">
          <div className="flex items-center justify-between gap-3"><div className="text-[13px] font-medium">mrd-service</div><Action disabled={!native || health.loading || quit.loading} onClick={() => void health.refresh()}>刷新</Action></div>
          <p className="mt-1 text-xs text-gray-500">后台服务的实际运行状态{health.error && health.value ? '（上次读取）' : ''}</p>
          <dl className="mt-4 grid grid-cols-3 gap-3 text-center">
            {[['运行状态', health.value ? health.value.running ? '运行中' : '未运行' : '未知'], ['健康检查', health.value ? health.value.healthy ? '健康' : '异常' : '未知'], ['进程 PID', health.value?.pid ? String(health.value.pid) : '—']].map(([label, value]) => <div key={label}><dd className="text-sm font-semibold">{value}</dd><dt className="mt-1 text-[11px] text-gray-500">{label}</dt></div>)}
          </dl><Feedback {...health} />
          <div className="mt-3 flex flex-wrap gap-2">
            <Action disabled={health.loading || quit.loading || !health.value || health.value.running} onClick={() => void runServiceAction(serviceStart)}>启动</Action>
            <Action disabled={health.loading || quit.loading || !health.value?.running} onClick={() => void runServiceAction(serviceStop)}>停止</Action>
            <Action disabled={health.loading || quit.loading || !health.value?.running} onClick={() => void runServiceAction(serviceRestart)}>重启</Action>
          </div>
        </div>
        <div className="mt-3 rounded-xl border border-gray-500/20 p-3.5"><h4 className="text-[13px]">网络检测</h4><dl className="mt-3 grid grid-cols-3 gap-3 text-center">{['延迟', '下载速度', '上传速度'].map(label => <div key={label}><dd className="text-sm text-gray-500">未测量</dd><dt className="mt-1 text-[11px] text-gray-500">{label}</dt></div>)}</dl></div>
        <Unsupported label="使用代理" toggle /><Unsupported label="优先直连" description="连接路径由会话服务选择；此处尚未提供策略设置" toggle /><Unsupported label="带宽限制" />
        {native && <ConnectionSummary />}
      </>}
      {active === 'display' && <>
        <h4 className="mt-4 text-[13px] font-medium">媒体解码</h4>
        <Row label="解码策略" description="当前版本尚未接入服务端解码策略设置">
          <select aria-label="解码策略" className={inputStyle} value={decode.value?.decode_policy ?? 'auto'} disabled>
            {['auto', 'software', 'd3d11va', 'nvdec'].map(policy => <option key={policy}>{policy}</option>)}
          </select>
        </Row>
        {decode.loading && <p role="status" className="mt-2 text-xs text-gray-500">正在读取解码策略…</p>}
        {decode.error && <p className="mt-2 break-words text-xs text-gray-500">读取状态：{decode.error}</p>}
        <div className="mt-4 rounded-xl border border-gray-500/20 p-3.5">
          <div className="flex flex-wrap items-center justify-between gap-2"><h4 className="text-[13px] font-medium">FFmpeg 可选工具</h4><span className={`text-xs ${ffmpeg.value?.available ? 'text-green-600' : 'text-gray-500'}`}>{ffmpeg.value ? ffmpeg.value.available ? 'FFmpeg 可用' : 'FFmpeg 未就绪' : '未探测'}</span></div>
          <dl className="mt-3 space-y-2 text-xs">
            <div><dt className="text-gray-500">FFmpeg 版本</dt><dd className="mt-1 break-words">{ffmpeg.value?.ffmpeg_version ?? '未探测'}</dd></div>
            <div><dt className="text-gray-500">路径</dt><dd className="mt-1 break-all font-mono">{ffmpeg.value?.ffmpeg_path ?? '未配置'}</dd></div>
            <div><dt className="text-gray-500">FFprobe 版本</dt><dd className="mt-1 break-words">{ffmpeg.value?.ffprobe_version ?? '未探测'}</dd></div>
          </dl>
          {ffmpeg.value?.reason && <p className="mt-2 break-words text-xs text-gray-500">{ffmpeg.value.reason}</p>}
          <div className="mt-3 flex flex-wrap gap-2">
            <Action label="刷新 FFmpeg 状态" disabled={!native || ffmpeg.loading} onClick={() => void ffmpeg.refresh()}>刷新探测</Action>
            <Action disabled={!native || ffmpeg.loading} onClick={() => void ffmpeg.run(async () => (await ffmpegDownload()).probe, 'FFmpeg 已下载并完成探测')}>下载或更新 FFmpeg</Action>
            <Action disabled={!native || ffmpeg.loading} onClick={() => void ffmpeg.run(async () => { await ffmpegResetGoldenSettings(); return ffmpegProbe(); }, 'FFmpeg 默认配置已恢复')}>恢复 FFmpeg 默认配置</Action>
          </div><Feedback {...ffmpeg} />
        </div>
        <Unsupported label="分辨率" description="会话建立时选择" /><Unsupported label="画质与色深" /><Unsupported label="远程光标" toggle />
      </>}
      {active === 'audio' && <><Unsupported label="远程音频输出" toggle /><Unsupported label="麦克风输入" toggle /><Unsupported label="键盘与鼠标默认策略" description="当前支持情况取决于远端设备与会话权限" /></>}
      {active === 'notifications' && <><Unsupported label="连接通知" toggle /><Unsupported label="断开通知" toggle /><Unsupported label="连接请求通知" toggle /><Unsupported label="提示音" toggle /></>}
      {active === 'account' && <>
        <div className="my-4 flex items-center gap-3"><div className="flex h-10 w-10 items-center justify-center rounded-full bg-blue-500/10 text-blue-500"><User className="h-5 w-5" /></div><div><p className="text-sm font-medium">{isLoggedIn ? user?.username : '未登录'}</p><p className="mt-1 text-xs text-gray-500">{isLoggedIn ? user?.role : '登录后可使用账户服务'}</p></div></div>
        <Unsupported label="升级账户" /><Unsupported label="修改密码" /><Unsupported label="注销账户" />
        {isLoggedIn && <div className="mt-5"><Action onClick={logout}><span className="flex items-center gap-2"><LogOut className="h-3.5 w-3.5" />退出登录</span></Action><p className="mt-2 text-xs text-gray-500">退出此设备的账户登录，保留本地偏好。</p></div>}
        {logoutError && <p role="alert" className="mt-3 break-words text-sm text-red-500">{logoutError}</p>}
      </>}
    </section>
  </div>;
}
