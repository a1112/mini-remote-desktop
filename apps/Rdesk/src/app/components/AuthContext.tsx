import { createContext, useContext, useState, useEffect, useCallback, type ReactNode } from "react";
import { closeAllBrowserRemoteSessions } from '../services/browserRemoteSessionService';
import { SERVER_API_URL } from '../services/serverConfig';
import { isTauriRuntime } from '../utils/runtime';

interface UserInfo {
  id: string;
  username: string;
  role: string;
}

interface AuthContextType {
  isLoggedIn: boolean;
  user: UserInfo | null;
  token: string | null;
  login: (token: string, user: UserInfo) => void;
  logout: () => void;
  logoutError: string | null;
}

const AuthContext = createContext<AuthContextType>({
  isLoggedIn: false,
  user: null,
  token: null,
  login: () => {},
  logout: () => {},
  logoutError: null,
});

export function useAuth() {
  return useContext(AuthContext);
}

const TOKEN_KEY = "rdesk_access_token";
const USER_KEY = "rdesk_auth_user";

function getStoredAuth(): { token: string | null; user: UserInfo | null } {
  try {
    const token = localStorage.getItem(TOKEN_KEY);
    const userStr = localStorage.getItem(USER_KEY);
    const user = userStr ? JSON.parse(userStr) : null;
    return { token, user };
  } catch {
    return { token: null, user: null };
  }
}

export function AuthProvider({ children }: { children: ReactNode }) {
  const [isLoggedIn, setIsLoggedIn] = useState(false);
  const [user, setUser] = useState<UserInfo | null>(null);
  const [token, setToken] = useState<string | null>(null);
  const [logoutError, setLogoutError] = useState<string | null>(null);

  // 初始化：检查本地存储的登录状态
  useEffect(() => {
    const { token: storedToken, user: storedUser } = getStoredAuth();
    if (storedToken && storedUser) {
      setToken(storedToken);
      setUser(storedUser);
      setIsLoggedIn(true);
    }
  }, []);

  // 监听登录状态变化事件（来自 AuthModal）
  useEffect(() => {
    const handleAuthChange = () => {
      const { token: storedToken, user: storedUser } = getStoredAuth();
      if (storedToken && storedUser) {
        setToken(storedToken);
        setUser(storedUser);
        setIsLoggedIn(true);
      } else {
        setToken(null);
        setUser(null);
        setIsLoggedIn(false);
      }
    };

    window.addEventListener("rdesk-auth-changed", handleAuthChange);
    return () => window.removeEventListener("rdesk-auth-changed", handleAuthChange);
  }, []);

  const login = useCallback((newToken: string, newUser: UserInfo) => {
    setLogoutError(null);
    localStorage.setItem(TOKEN_KEY, newToken);
    localStorage.setItem(USER_KEY, JSON.stringify(newUser));
    setToken(newToken);
    setUser(newUser);
    setIsLoggedIn(true);
  }, []);

  const logout = useCallback(() => {
    const capturedToken = localStorage.getItem(TOKEN_KEY) ?? token;
    setLogoutError(null);
    localStorage.removeItem(TOKEN_KEY);
    localStorage.removeItem(USER_KEY);
    setToken(null);
    setUser(null);
    setIsLoggedIn(false);
    window.dispatchEvent(new Event("rdesk-auth-changed"));
    if (!isTauriRuntime()) {
      void closeAllBrowserRemoteSessions('logout').catch(error => {
        if (!localStorage.getItem(TOKEN_KEY)) setLogoutError(`本地已退出，远端会话结束请求失败：${error instanceof Error ? error.message : String(error)}`);
      });
      if (capturedToken) {
        void fetch(`${SERVER_API_URL}/auth/logout`, {
          method: 'POST',
          headers: { Authorization: `Bearer ${capturedToken}`, 'Content-Type': 'application/json' },
          body: '{}',
        }).then(response => {
          if (!response.ok) throw new Error(`HTTP ${response.status}`);
        }).catch(error => {
          if (!localStorage.getItem(TOKEN_KEY)) setLogoutError(`本地已退出，服务端撤销失败：${error instanceof Error ? error.message : String(error)}。无法确认远端已即时停止。`);
        });
      }
    }
  }, [token]);

  return (
    <AuthContext.Provider value={{ isLoggedIn, user, token, login, logout, logoutError }}>
      {children}
    </AuthContext.Provider>
  );
}
