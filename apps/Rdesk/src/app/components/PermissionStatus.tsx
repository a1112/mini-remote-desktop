import { useEffect, useRef, useState } from 'react';
import { Loader2, ShieldCheck } from 'lucide-react';
import { ipcCapabilitySnapshot, shellGetStatus, openMacosPermissionSettings } from '../adapters/tauri/commands';
import type { CapabilitySnapshot, ShellStatusSnapshot } from '../adapters/tauri/types';
import { useTheme } from './ThemeContext';
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover';

const MAX_REFRESH_READS = 3;
const REFRESH_DELAY_MS = 1_000;
const MAX_SNAPSHOT_AGE_MS = 30_000;
const hasNativeIpc = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

type PermissionLabel = '可用' | '待授权' | '待检查' | '受限' | '暂不支持' | '未能确认';
type PermissionRow = { name: string; label: PermissionLabel; description: string; settings: 'screen_capture' | 'accessibility' };

function validSnapshot(value: unknown): value is CapabilitySnapshot {
  if (!value || typeof value !== 'object') return false;
  const candidate = value as CapabilitySnapshot;
  return candidate.schema_version === 1 && ['windows', 'macos', 'linux', 'android', 'ios'].includes(candidate.platform)
    && Number.isSafeInteger(candidate.updated_at_ms) && candidate.updated_at_ms > 0
    && Array.isArray(candidate.capabilities)
    && candidate.capabilities.every(item => item && typeof item.id === 'string' && typeof item.status === 'string');
}

function permissionRows(snapshot: CapabilitySnapshot | null, now: number): PermissionRow[] {
  const fresh = snapshot && now - snapshot.updated_at_ms < MAX_SNAPSHOT_AGE_MS && snapshot.updated_at_ms <= now + 5_000;
  const captureId = snapshot?.platform === 'macos' ? 'capture.macos'
    : snapshot?.platform === 'windows' ? (snapshot.capabilities.some(item => item.id === 'capture.dxgi') ? 'capture.dxgi' : 'capture.winrt')
      : snapshot?.platform === 'linux' ? 'capture.linux' : null;
  return [{ name: '录屏', id: captureId, domain: 'capture' }, { name: '键鼠', id: 'control.keyboard_mouse', domain: 'control' }].map(({ name, id, domain }) => {
    const matches = fresh && id ? snapshot.capabilities.filter(item => item.id === id) : [];
    const item = matches.length === 1 ? matches[0] : undefined;
    const status = item?.platform === snapshot?.platform && item?.domain === domain ? item.status : 'unknown';
    const label: PermissionLabel = status === 'available' || status === 'usable' ? '可用'
      : status === 'permission_missing' ? '待授权' : status === 'supported' ? '待检查'
        : status === 'degraded' ? '受限' : status === 'unsupported' || status === 'unimplemented' ? '暂不支持' : '未能确认';
    const description = label === '待授权' ? name === '录屏' ? '录屏权限缺失'
      : snapshot?.platform === 'macos' ? '辅助功能或事件控制权限缺失' : '键鼠控制权限缺失'
      : label === '可用' ? `当前本机服务报告${name}能力可用。`
        : label === '待检查' ? '已支持此能力，尚未确认当前运行中的权限。'
          : label === '受限' ? '当前本机服务报告此能力受限。'
            : label === '暂不支持' ? '当前本机服务暂不支持此能力。' : '未能确认当前本机服务的权限。';
    return { name, label, description, settings: name === '录屏' ? 'screen_capture' : 'accessibility' };
  });
}

export function PermissionStatus() {
  // Browser controllers have no local service requirement, including pages with a bridge configured.
  if (!hasNativeIpc()) return null;
  return <NativePermissionStatus />;
}

function NativePermissionStatus() {
  const { isDark } = useTheme();
  const [snapshot, setSnapshot] = useState<CapabilitySnapshot | null>(null);
  const [service, setService] = useState<ShellStatusSnapshot | null>(null);
  const [settingsError, setSettingsError] = useState(false);
  const [openingSettings, setOpeningSettings] = useState(false);
  const settingsPending = useRef(false);
  const mounted = useRef(true);
  const [readFailed, setReadFailed] = useState(false);
  const [checking, setChecking] = useState(true);
  const [now, setNow] = useState(Date.now());
  const refreshRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    let stopped = false;
    mounted.current = true;
    let running = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let finishDelay: (() => void) | undefined;
    const refresh = async () => {
      if (stopped || running || !hasNativeIpc()) return;
      running = true;
      setChecking(true); setReadFailed(false); setSnapshot(null); setService(null);
      let firstUpdatedAt: number | undefined;
      try {
        for (let attempt = 0; attempt < MAX_REFRESH_READS; attempt += 1) {
          if (stopped || !hasNativeIpc()) return;
          const [result, identity] = await Promise.all([
            ipcCapabilitySnapshot(),
            attempt === 0 ? shellGetStatus().catch(() => null) : Promise.resolve(null),
          ]);
          if (stopped) return;
          if (identity?.ok && Number.isSafeInteger(identity.value.service_pid) && identity.value.service_pid > 0) {
            setService(identity.value);
          }
          if (!result.ok || !validSnapshot(result.value)) {
            setSnapshot(null); setReadFailed(true); break;
          }
          setNow(Date.now()); setSnapshot(result.value);
          firstUpdatedAt ??= result.value.updated_at_ms;
          // The first read returns a cache and triggers the service's background probe.
          const hasPending = permissionRows(result.value, Date.now()).some(row => row.label === '待检查');
          if (attempt > 0 && result.value.updated_at_ms > firstUpdatedAt && !hasPending) break;
          if (attempt === MAX_REFRESH_READS - 1) break;
          await new Promise<void>(resolve => {
            finishDelay = resolve;
            timer = setTimeout(() => { timer = undefined; finishDelay = undefined; resolve(); }, REFRESH_DELAY_MS);
          });
        }
      } catch {
        if (!stopped) { setSnapshot(null); setReadFailed(true); }
      } finally {
        running = false;
        if (!stopped) setChecking(false);
      }
    };
    const requestRefresh = () => { void refresh(); };
    const visible = () => { if (!document.hidden) requestRefresh(); };
    refreshRef.current = requestRefresh;
    window.addEventListener('focus', requestRefresh);
    document.addEventListener('visibilitychange', visible);
    requestRefresh();
    return () => {
      stopped = true;
      mounted.current = false;
      if (timer !== undefined) clearTimeout(timer);
      finishDelay?.();
      refreshRef.current = null;
      window.removeEventListener('focus', requestRefresh);
      document.removeEventListener('visibilitychange', visible);
    };
  }, []);

  useEffect(() => {
    if (!snapshot) return;
    const delay = snapshot.updated_at_ms + MAX_SNAPSHOT_AGE_MS - Date.now();
    if (delay <= 0) { setNow(Date.now()); return; }
    const timer = setTimeout(() => setNow(Date.now()), delay);
    return () => clearTimeout(timer);
  }, [snapshot]);

  const openSettings = async (permission: 'screen_capture' | 'accessibility') => {
    if (settingsPending.current) return;
    settingsPending.current = true;
    setOpeningSettings(true); setSettingsError(false);
    try {
      const result = await openMacosPermissionSettings(permission);
      if (mounted.current && !result.ok) setSettingsError(true);
    } catch {
      if (mounted.current) setSettingsError(true);
    } finally {
      settingsPending.current = false;
      if (mounted.current) setOpeningSettings(false);
    }
    // Rechecks occur on focus/visibility return or explicit refresh. Opening
    // Settings cannot change a service capability to available by itself.
  };
  const servicePath = typeof service?.service_executable_path === 'string'
    && service.service_executable_path.startsWith('/') ? service.service_executable_path : null;

  const rows = permissionRows(snapshot, now);
  const summary = rows.some(row => row.label === '待授权') ? '待授权'
    : rows.every(row => row.label === '可用') ? '可用'
      : rows.some(row => row.label === '待检查') ? '待检查'
        : rows.some(row => row.label === '受限') ? '受限'
          : rows.every(row => row.label === '暂不支持') ? '暂不支持' : '未能确认';
  const color = (label: PermissionLabel) => label === '可用' ? 'text-emerald-600'
    : label === '待授权' || label === '受限' ? 'text-amber-600' : isDark ? 'text-gray-400' : 'text-gray-500';
  const accessibleName = `本机系统权限：${rows.map(row => row.name + row.label).join('，')}`;

  return <Popover onOpenChange={(open: boolean) => { if (open) refreshRef.current?.(); }}>
    <PopoverTrigger asChild>
      <button type="button" aria-label={accessibleName} title={accessibleName} data-no-drag="true"
        className={`flex h-full shrink-0 items-center gap-1.5 px-2 transition-colors ${isDark ? 'hover:bg-gray-700' : 'hover:bg-gray-100'}`}
        style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
        {checking ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldCheck className={`h-3.5 w-3.5 ${color(summary)}`} />}
        <span className={`text-[11px] xl:hidden ${color(summary)}`}>权限：{summary}</span>
        <span className="hidden text-[11px] xl:flex xl:flex-col xl:items-start">
          {rows.map(row => <span key={row.name} className={color(row.label)}>{row.name}：{row.label}</span>)}
        </span>
      </button>
    </PopoverTrigger>
    <PopoverContent align="end" role="dialog" aria-label="本机系统权限详情" data-no-drag="true"
      className={`w-[360px] max-h-[70vh] overflow-y-auto max-w-[calc(100vw-24px)] text-xs ${isDark ? 'border-gray-700 bg-[#1e1e1e] text-gray-200' : 'border-gray-200 bg-white text-gray-800'}`}
      style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
      <h2 className="mb-3 font-medium">本机系统权限</h2>
      <dl className="space-y-3">{rows.map(row => <div key={row.name}>
        <div className="flex items-center justify-between"><dt>{row.name}</dt><dd className={color(row.label)}>{row.label}</dd></div>
        <p className="mt-1 leading-relaxed opacity-70">{row.description}</p>
        {snapshot?.platform === 'macos' && row.label !== '暂不支持' && <button type="button"
          disabled={openingSettings} onClick={() => void openSettings(row.settings)}
          className="mt-2 rounded border border-current/20 px-2 py-1 disabled:opacity-50">
          {row.settings === 'screen_capture' ? '打开录屏设置' : '打开辅助功能设置'}
        </button>}
      </div>)}</dl>
      {snapshot?.platform === 'macos' && <div className="mt-3 space-y-2 leading-relaxed">
        <p>需要授权的是运行中的 mrd-service（打包名称 Rdesk Service）。仅授权 Rdesk 界面不能证明服务已获授权。</p>
        {servicePath ? <p className="break-all">服务程序：{servicePath}（PID {service?.service_pid}）</p>
          : <p>未能确认服务程序路径。请重新检查；不要根据界面程序的位置猜测服务位置。</p>}
        <p>系统设置 → 隐私与安全性 → 屏幕录制（部分系统称“屏幕与系统音频录制”）或辅助功能。核对上述服务程序对应的条目；需要添加时请选择实际安装的服务。系统可能按负责的应用归类，最终以服务重新检查结果为准。</p>
        <p>录屏用于共享本机画面；辅助功能用于允许远程键鼠控制。只观看远端画面无需为本机授予这两项权限。</p>
        <p>修改后返回此处会重新检查。若系统要求退出并重新打开，请先结束会话并保存工作，再退出界面和服务后重开；仅关闭窗口可能保留服务。缺少权限本身不代表必须重启。</p>
      </div>}
      {settingsError && <p role="status" className="mt-3 text-amber-600">未能打开系统设置，请按照上面的路径手动打开。尚未确认权限发生变化。</p>}
      {readFailed && <p role="status" className="mt-3 text-amber-600">未能读取本机权限状态，请重新检查。</p>}
      {snapshot && <p className="mt-3 opacity-60">快照更新时间：{new Date(snapshot.updated_at_ms).toLocaleTimeString('zh-CN')}</p>}
      <p className="mt-3 leading-relaxed opacity-60">此处显示本机服务的能力状态；每次连接的应用授权仍需单独确认。</p>
      <button type="button" aria-label="重新检查本机权限" disabled={checking} onClick={() => refreshRef.current?.()}
        className="mt-3 rounded border border-current/20 px-3 py-1.5 disabled:opacity-50">{checking ? '正在检查…' : '重新检查'}</button>
    </PopoverContent>
  </Popover>;
}
