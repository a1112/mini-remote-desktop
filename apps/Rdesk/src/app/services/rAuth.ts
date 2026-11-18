import { openRAuthLogin } from '../adapters/tauri/commands';
import { isTauriRuntime } from '../utils/runtime';
import { SERVER_API_URL } from './serverConfig';

function secureApi(): string | null {
  try {
    const url = new URL(SERVER_API_URL);
    if (url.protocol === 'https:' && !url.username && !url.password && !url.search && !url.hash) {
      return `${url.href.replace(/\/+$/, '')}/auth/oidc`;
    }
  } catch {
    return null;
  }
  return null;
}

type LoginResponse = {
  access_token: string;
  user_id: string;
  username: string;
  role: string;
};

export async function rAuthAvailable(signal: AbortSignal): Promise<boolean> {
  const api = secureApi();
  if (!api) return false;
  const result = await fetch(`${api}/capabilities`, {
    credentials: 'omit', cache: 'no-store', redirect: 'error', signal,
  });
  if (!result.ok) return false;
  return (await result.json())?.enabled === true;
}

function waitForNextPoll(signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    signal.throwIfAborted();
    const cancel = () => { clearTimeout(timer); reject(signal.reason); };
    const timer = setTimeout(() => {
      signal.removeEventListener('abort', cancel);
      resolve();
    }, 1000);
    signal.addEventListener('abort', cancel, { once: true });
  });
}

export async function signInWithRAuth(
  signal: AbortSignal,
  onBrowserOpened: () => void,
  existingToken?: string,
): Promise<LoginResponse> {
  const api = secureApi();
  if (!api) throw new Error('R-AUTH 登录需要 HTTPS 服务地址');
  signal.throwIfAborted();
  const native = isTauriRuntime();
  const popup = native ? null : window.open('about:blank', '_blank');
  if (!native && !popup) throw new Error('浏览器拦截了登录窗口，请允许此网站弹出窗口后重试');
  if (popup) popup.opener = null;

  const timer = new AbortController();
  const cancel = () => timer.abort(signal.reason);
  signal.addEventListener('abort', cancel, { once: true });
  let deadline = setTimeout(() => timer.abort(new Error('R-AUTH 登录超时，请重试')), 15000);
  const post = (path: string, data: object, token?: string) => fetch(`${api}/${path}`, {
    method: 'POST', credentials: 'omit', cache: 'no-store', redirect: 'error', signal: timer.signal,
    headers: { 'Content-Type': 'application/json', ...(token ? { Authorization: `Bearer ${token}` } : {}) },
    body: JSON.stringify(data),
  });
  try {
    const started = await post(existingToken ? 'link/start' : 'start', {}, existingToken);
    if (!started.ok) throw new Error('无法发起 R-AUTH 登录，请重试');
    const { authorization_url, completion_token, expires_in } = await started.json();
    if (typeof authorization_url !== 'string' || authorization_url.length > 8192 ||
        typeof completion_token !== 'string' || !completion_token || completion_token.length > 512 ||
        !Number.isSafeInteger(expires_in) || expires_in < 1 || expires_in > 600) {
      throw new Error('R-AUTH 返回的登录信息无效');
    }
    let authorization: URL;
    try {
      authorization = new URL(authorization_url);
    } catch {
      throw new Error('R-AUTH 登录地址无效');
    }
    if (authorization.protocol !== 'https:' || authorization.username || authorization.password || authorization.hash) {
      throw new Error('R-AUTH 登录地址无效');
    }
    clearTimeout(deadline);
    deadline = setTimeout(() => timer.abort(new Error('R-AUTH 登录超时，请重试')), expires_in * 1000);
    timer.signal.throwIfAborted();
    if (popup) popup.location.replace(authorization.href);
    else {
      const opened = await openRAuthLogin(authorization.href);
      if (!opened.ok) throw new Error(opened.error.message);
    }
    timer.signal.throwIfAborted();
    onBrowserOpened();
    for (;;) {
      await waitForNextPoll(timer.signal);
      const completed = await post('complete', { completion_token }, existingToken);
      if (completed.status === 202) continue;
      if (!completed.ok) throw new Error('R-AUTH 未完成登录或账号关联，请重试');
      const user: LoginResponse = await completed.json();
      if (![user?.access_token, user?.user_id, user?.username, user?.role].every(value => typeof value === 'string' && value.length)) {
        throw new Error('R-AUTH 返回的账号会话无效');
      }
      return user;
    }
  } catch (error) {
    if (timer.signal.aborted) throw timer.signal.reason;
    throw error;
  } finally {
    clearTimeout(deadline);
    signal.removeEventListener('abort', cancel);
    popup?.close();
  }
}
