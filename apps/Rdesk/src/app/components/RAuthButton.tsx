import { useEffect, useRef, useState } from 'react';
import { useAuth } from './AuthContext';
import { deviceService } from '../services/deviceService';
import { rAuthAvailable, signInWithRAuth } from '../services/rAuth';
import { isTauriRuntime } from '../utils/runtime';

export function RAuthButton({
  linkExisting = false, disabled = false, onAuthenticated, onBusyChange,
}: {
  linkExisting?: boolean;
  disabled?: boolean;
  onAuthenticated?: () => void;
  onBusyChange?: (busy: boolean) => void;
}) {
  const { token, user, login } = useAuth();
  const currentToken = useRef(token);
  currentToken.current = token;
  const attempt = useRef<AbortController | null>(null);
  const [available, setAvailable] = useState(false);
  const [status, setStatus] = useState<'idle' | 'starting' | 'waiting' | 'complete'>('idle');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const capability = new AbortController();
    void rAuthAvailable(capability.signal).then(enabled => {
      if (!capability.signal.aborted) setAvailable(enabled);
    }).catch(() => undefined);
    return () => {
      capability.abort();
      attempt.current?.abort();
      if (attempt.current) onBusyChange?.(false);
    };
  }, [onBusyChange]);

  async function authenticate() {
    if (attempt.current || (linkExisting && !token)) return;
    const active = new AbortController();
    attempt.current = active;
    onBusyChange?.(true);
    setError(null);
    setStatus('starting');
    try {
      const result = await signInWithRAuth(active.signal, () => setStatus('waiting'), linkExisting ? token ?? undefined : undefined);
      if (active.signal.aborted || currentToken.current !== token) return;
      if (linkExisting && result.user_id !== user?.id) throw new Error('R-AUTH 返回的账号与当前账号不一致');
      login(result.access_token, { id: result.user_id, username: result.username, role: result.role });
      const binding = isTauriRuntime() ? await deviceService.bindDevice() : undefined;
      if (active.signal.aborted) return;
      if (linkExisting || binding?.success === false) setStatus('complete');
      if (binding?.success === false) {
        setError(binding.message);
        return;
      }
      onAuthenticated?.();
    } catch (error) {
      if (!active.signal.aborted) setError(error instanceof Error ? error.message : 'R-AUTH 登录失败');
    } finally {
      attempt.current = null;
      if (!active.signal.aborted) {
        onBusyChange?.(false);
        setStatus(current => current === 'complete' ? current : 'idle');
      }
    }
  }

  if (!available || (linkExisting && !token)) return null;
  return <div className="space-y-2">
    <button type="button" onClick={() => void authenticate()}
      disabled={disabled || status !== 'idle'}
      className="w-full rounded-lg border border-blue-500/50 px-3 py-2 text-sm font-medium text-blue-600 hover:bg-blue-500/10 disabled:opacity-60 dark:text-blue-400">
      {status === 'complete' ? (linkExisting ? 'R-AUTH 已关联' : 'R-AUTH 已登录') : status === 'starting' ? '正在打开登录窗口' :
        status === 'waiting' ? '请在浏览器完成 R-AUTH 登录' : linkExisting ? '关联 R-AUTH 账号' : '使用 R-AUTH 登录'}
    </button>
    {error && <p role="alert" className="text-xs text-red-500">{error}</p>}
  </div>;
}
