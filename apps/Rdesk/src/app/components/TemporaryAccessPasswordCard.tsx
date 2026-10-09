import { useEffect, useRef, useState } from 'react';
import { Check, Copy, Eye, EyeOff, Loader2, RefreshCw } from 'lucide-react';
import {
  disableTemporaryAccess, getTemporaryAccessStatus, readTemporaryAccessPassword, rotateTemporaryAccessPassword,
  type TemporaryAccessStatus,
} from '../services/temporaryAccessService';

export function TemporaryAccessPasswordCard() {
  const [status, setStatus] = useState<TemporaryAccessStatus | null>(null);
  const [secret, setSecret] = useState<{ value: string; expires: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [now, setNow] = useState(Date.now());
  const currentStatus = useRef<TemporaryAccessStatus | null>(null);
  const alive = useRef(true);
  const operation = useRef(false);
  const epoch = useRef(0);

  const applyStatus = (next: TemporaryAccessStatus) => {
    if (currentStatus.current?.generation !== next.generation || !next.ready || !next.enabled) {
      epoch.current += 1; setSecret(null); setCopied(false);
    }
    currentStatus.current = next; setStatus(next); setLoadError(false);
  };

  useEffect(() => {
    alive.current = true;
    const refresh = async () => {
      if (operation.current) return;
      const version = epoch.current;
      try {
        const next = await getTemporaryAccessStatus();
        if (alive.current && !operation.current && version === epoch.current) applyStatus(next);
      } catch { if (alive.current && version === epoch.current) { setLoadError(true); setSecret(null); } }
    };
    void refresh();
    const poll = setInterval(() => void refresh(), 5000);
    const timer = setInterval(() => setNow(Date.now()), 1000);
    const hide = () => { if (document.hidden) { epoch.current += 1; setSecret(null); setCopied(false); } };
    document.addEventListener('visibilitychange', hide);
    return () => { alive.current = false; epoch.current += 1; clearInterval(poll); clearInterval(timer); document.removeEventListener('visibilitychange', hide); };
  }, []);
  useEffect(() => { if (secret && secret.expires <= now) setSecret(null); }, [now, secret]);

  const ready = !!status?.enabled && status.ready && status.expires_at_ms !== null && status.expires_at_ms > now && !loadError;
  const seconds = ready ? Math.max(0, Math.ceil((status!.expires_at_ms! - now) / 1000)) : 0;
  const label = loadError ? '无法读取本机临时密码状态' : !status ? '正在读取临时密码状态' : !status.enabled ? '临时访问已关闭'
    : status.expires_at_ms !== null && status.expires_at_ms <= now ? '临时密码已过期' : ready ? '临时密码可用'
      : /offline|signaling/.test(status.reason ?? '') ? '公网连接未就绪' : '正在准备临时密码';

  const act = async (kind: 'reveal' | 'copy' | 'rotate' | 'disable') => {
    if (operation.current || document.hidden) return;
    operation.current = true; epoch.current += 1;
    const version = epoch.current;
    const expected = currentStatus.current;
    setBusy(true); setError(null); setCopied(false);
    if (kind === 'rotate' || kind === 'disable') setSecret(null);
    try {
      if (kind === 'rotate' || kind === 'disable') {
        const next = await (kind === 'rotate' ? rotateTemporaryAccessPassword() : disableTemporaryAccess());
        if (alive.current && version === epoch.current) applyStatus(next);
      } else {
        const reply = await readTemporaryAccessPassword();
        if (!alive.current || document.hidden || version !== epoch.current) return;
        if (!expected || !reply.password || !reply.status.ready || !reply.status.enabled
          || reply.status.generation !== expected.generation || reply.status.expires_at_ms === null || reply.status.expires_at_ms <= Date.now()) {
          applyStatus(reply.status); throw new Error('临时密码已变化或过期，请重试');
        }
        if (kind === 'copy') { await navigator.clipboard.writeText(reply.password); if (alive.current && version === epoch.current) setCopied(true); }
        else setSecret({ value: reply.password, expires: reply.status.expires_at_ms });
      }
    } catch (cause) {
      if (alive.current) { setSecret(null); setError(cause instanceof Error ? cause.message : '临时密码操作失败'); }
    } finally {
      operation.current = false;
      if (alive.current) setBusy(false);
    }
  };

  return <section aria-label="本机临时密码" className="mt-4 space-y-2 border-t border-current/10 pt-4">
    <div className="flex items-center justify-between gap-2"><h3 className="text-sm font-medium">临时密码</h3><span role="status" className={`text-xs ${ready ? 'text-emerald-600' : 'opacity-60'}`}>{label}</span></div>
    <div className="flex items-center gap-2">
      <code className="min-w-0 flex-1 text-lg tracking-widest">{ready ? secret?.value ?? '••••••••' : '—'}</code>
      <button type="button" disabled={!ready || busy} aria-label={secret ? '隐藏临时密码' : '显示临时密码'}
        onClick={() => secret ? setSecret(null) : void act('reveal')} className="rounded p-2 opacity-70 hover:bg-current/10 disabled:opacity-30">{secret ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}</button>
      <button type="button" disabled={!ready || busy} aria-label="复制临时密码" onClick={() => void act('copy')} className="rounded p-2 opacity-70 hover:bg-current/10 disabled:opacity-30">{copied ? <Check className="h-4 w-4 text-emerald-600" /> : <Copy className="h-4 w-4" />}</button>
      <button type="button" disabled={busy} aria-label="刷新临时密码" onClick={() => void act('rotate')} className="rounded p-2 opacity-70 hover:bg-current/10 disabled:opacity-30">{busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}</button>
    </div>
    <p className="text-xs leading-relaxed opacity-60">{ready ? `剩余 ${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}。` : ''}无需登录即可被控，连接时仍需本机确认权限。</p>
    <div className="flex items-center justify-between gap-2 text-xs"><span className="opacity-60">刷新后旧密码失效。</span>
      {status?.enabled ? <button type="button" disabled={busy} onClick={() => void act('disable')} className="rounded px-2 py-1 text-red-500 hover:bg-red-500/10 disabled:opacity-30">关闭临时访问</button>
        : <button type="button" disabled={busy} onClick={() => void act('rotate')} className="rounded px-2 py-1 text-blue-500 hover:bg-blue-500/10 disabled:opacity-30">启用临时访问</button>}
    </div>
    {error && <p role="alert" className="break-words text-xs text-red-500">{error}</p>}
  </section>;
}
