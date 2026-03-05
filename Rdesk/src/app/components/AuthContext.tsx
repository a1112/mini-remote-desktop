import { createContext, useContext, useState, useEffect, useCallback, type ReactNode } from "react";

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
}

const AuthContext = createContext<AuthContextType>({
  isLoggedIn: false,
  user: null,
  token: null,
  login: () => {},
  logout: () => {},
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
    localStorage.setItem(TOKEN_KEY, newToken);
    localStorage.setItem(USER_KEY, JSON.stringify(newUser));
    setToken(newToken);
    setUser(newUser);
    setIsLoggedIn(true);
  }, []);

  const logout = useCallback(() => {
    localStorage.removeItem(TOKEN_KEY);
    localStorage.removeItem(USER_KEY);
    setToken(null);
    setUser(null);
    setIsLoggedIn(false);
  }, []);

  return (
    <AuthContext.Provider value={{ isLoggedIn, user, token, login, logout }}>
      {children}
    </AuthContext.Provider>
  );
}
