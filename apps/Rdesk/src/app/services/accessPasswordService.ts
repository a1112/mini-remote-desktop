/**
 * 接入密码管理服务
 *
 * - 密码由服务器端生成
 * - 客户端明文存储在 localStorage
 * - 密码与设备绑定
 * - 支持按小时整点自动刷新
 */

const ACCESS_PASSWORD_KEY = "rdesk_access_password";
const REFRESH_MODE_KEY = "rdesk_password_refresh_mode";
const API_BASE = "http://127.0.0.1:9530/api/v1";

export type RefreshMode = "manual" | "once" | "hourly" | "daily";

export interface RefreshOption {
  key: RefreshMode;
  label: string;
  description: string;
}

export const REFRESH_OPTIONS: RefreshOption[] = [
  { key: "manual", label: "手动", description: "仅手动刷新密码" },
  { key: "once", label: "单次", description: "立即刷新一次，之后不再自动刷新" },
  { key: "hourly", label: "每小时", description: "每整点自动刷新密码" },
  { key: "daily", label: "每天", description: "每天 00:00 自动刷新密码" },
];

class AccessPasswordService {
  /**
   * 从本地存储获取接入密码
   */
  getPassword(): string | null {
    try {
      return localStorage.getItem(ACCESS_PASSWORD_KEY);
    } catch {
      return null;
    }
  }

  /**
   * 保存接入密码到本地存储
   */
  savePassword(password: string): void {
    try {
      localStorage.setItem(ACCESS_PASSWORD_KEY, password);
    } catch (err) {
      console.warn("[AccessPasswordService] 保存密码失败:", err);
    }
  }

  /**
   * 从服务器生成新的接入密码
   */
  async generateNewPassword(): Promise<string> {
    const response = await fetch(`${API_BASE}/devices/access-password/generate`, {
      method: "POST",
    });

    if (!response.ok) {
      throw new Error(`生成密码失败: ${response.status}`);
    }

    const data = await response.json();
    const newPassword = data.password;

    // 保存到本地存储
    this.savePassword(newPassword);

    return newPassword;
  }

  /**
   * 获取或初始化接入密码
   * 如果本地没有密码，则从服务器生成新的
   */
  async getOrInitializePassword(): Promise<string> {
    let password = this.getPassword();

    if (!password) {
      // 本地没有密码，从服务器获取/生成
      try {
        const response = await fetch(`${API_BASE}/devices/access-password`, {
          method: "GET",
        });

        if (response.ok) {
          const data = await response.json();
          password = data.password;
          this.savePassword(password);
        } else {
          // 如果获取失败，生成新密码
          password = await this.generateNewPassword();
        }
      } catch (err) {
        // 网络错误，生成一个临时密码
        console.warn("[AccessPasswordService] 获取密码失败，使用临时密码:", err);
        password = this.generateTemporaryPassword();
        this.savePassword(password);
      }
    }

    return password;
  }

  /**
   * 生成临时密码（仅用于离线场景）
   */
  private generateTemporaryPassword(): string {
    const chars = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // 排除易混淆字符
    let password = "";
    for (let i = 0; i < 8; i++) {
      password += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    return password;
  }

  /**
   * 清除本地存储的密码
   */
  clearPassword(): void {
    try {
      localStorage.removeItem(ACCESS_PASSWORD_KEY);
    } catch (err) {
      console.warn("[AccessPasswordService] 清除密码失败:", err);
    }
  }

  /**
   * 生成临时密码（公开方法，供外部调用）
   */
  generateTemporaryPassword(): string {
    const chars = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // 排除易混淆字符
    let password = "";
    for (let i = 0; i < 8; i++) {
      password += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    return password;
  }

  /**
   * 获取刷新模式
   */
  getRefreshMode(): RefreshMode {
    try {
      const mode = localStorage.getItem(REFRESH_MODE_KEY);
      return (mode as RefreshMode) || "manual";
    } catch {
      return "manual";
    }
  }

  /**
   * 设置刷新模式
   */
  setRefreshMode(mode: RefreshMode): void {
    try {
      localStorage.setItem(REFRESH_MODE_KEY, mode);
    } catch (err) {
      console.warn("[AccessPasswordService] 设置刷新模式失败:", err);
    }
  }

  /**
   * 从服务器获取当前设备的接入密码
   * 密码与设备绑定
   */
  async fetchDevicePassword(deviceId: string | null): Promise<string> {
    const url = deviceId
      ? `${API_BASE}/devices/${deviceId}/access-password`
      : `${API_BASE}/devices/access-password`;

    try {
      const response = await fetch(url, {
        method: "GET",
      });

      if (!response.ok) {
        // 404 或其他错误，降级到本地生成
        console.warn("[AccessPasswordService] API 不可用，使用本地生成密码");
        return this.generateTemporaryPassword();
      }

      const data = await response.json();
      const newPassword = data.password;

      // 保存到本地存储
      this.savePassword(newPassword);

      return newPassword;
    } catch (err) {
      // 网络错误，降级到本地生成
      console.warn("[AccessPasswordService] 网络错误，使用本地生成密码:", err);
      return this.generateTemporaryPassword();
    }
  }

  /**
   * 刷新接入密码（从服务器获取最新密码）
   */
  async refreshPassword(deviceId: string | null): Promise<string> {
    return this.fetchDevicePassword(deviceId);
  }

  /**
   * 计算下次刷新时间（整点）
   */
  getNextHourlyRefreshTime(): Date {
    const now = new Date();
    const nextHour = new Date(now);
    nextHour.setHours(now.getHours() + 1, 0, 0, 0);
    return nextHour;
  }

  /**
   * 计算下次每日刷新时间（00:00）
   */
  getNextDailyRefreshTime(): Date {
    const now = new Date();
    const tomorrow = new Date(now);
    tomorrow.setDate(now.getDate() + 1);
    tomorrow.setHours(0, 0, 0, 0);
    return tomorrow;
  }

  /**
   * 检查是否需要刷新（基于整点条件）
   */
  shouldRefreshForHourly(): boolean {
    const now = new Date();
    const minutes = now.getMinutes();
    const seconds = now.getSeconds();
    // 在整点前后30秒内触发（允许一些延迟）
    return minutes === 0 && seconds < 30;
  }

  /**
   * 检查是否需要刷新（基于每日00:00条件）
   */
  shouldRefreshForDaily(): boolean {
    const now = new Date();
    const hours = now.getHours();
    const minutes = now.getMinutes();
    const seconds = now.getSeconds();
    // 在00:00前后30秒内触发
    return hours === 0 && minutes === 0 && seconds < 30;
  }
}

// 导出单例
export const accessPasswordService = new AccessPasswordService();

// React Hook
import { useState, useEffect, useRef, useCallback } from "react";

export function useAccessPassword(deviceId: string | null = null) {
  const [password, setPassword] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshMode, setRefreshMode] = useState<RefreshMode>(() =>
    accessPasswordService.getRefreshMode()
  );
  const onceRefreshedRef = useRef(false);

  // 刷新密码
  const refreshPassword = useCallback(async () => {
    setRefreshing(true);
    try {
      const newPassword = await accessPasswordService.refreshPassword(deviceId);
      setPassword(newPassword);
      return newPassword;
    } catch (err) {
      console.error("[useAccessPassword] 刷新密码失败:", err);
      // 即使失败也生成一个临时密码
      const tempPassword = accessPasswordService.generateTemporaryPassword();
      setPassword(tempPassword);
      return tempPassword;
    } finally {
      setRefreshing(false);
    }
  }, [deviceId]);

  // 初始化加载
  useEffect(() => {
    accessPasswordService.getOrInitializePassword().then((pwd) => {
      setPassword(pwd);
      setLoading(false);
    });
  }, []);

  // 根据刷新模式自动刷新
  useEffect(() => {
    if (refreshMode === "manual" || loading || refreshing) return;

    // 单次刷新
    if (refreshMode === "once" && !onceRefreshedRef.current) {
      onceRefreshedRef.current = true;
      refreshPassword();
      return;
    }

    // 每小时刷新（整点触发）
    if (refreshMode === "hourly") {
      const interval = setInterval(() => {
        if (accessPasswordService.shouldRefreshForHourly()) {
          console.log("[useAccessPassword] 整点刷新密码");
          refreshPassword();
        }
      }, 30000); // 每30秒检查一次
      return () => clearInterval(interval);
    }

    // 每日刷新（00:00触发）
    if (refreshMode === "daily") {
      const interval = setInterval(() => {
        if (accessPasswordService.shouldRefreshForDaily()) {
          console.log("[useAccessPassword] 每日刷新密码");
          refreshPassword();
        }
      }, 30000); // 每30秒检查一次
      return () => clearInterval(interval);
    }
  }, [refreshMode, loading, refreshing, refreshPassword]);

  const generateNew = async () => {
    const newPassword = await accessPasswordService.generateNewPassword();
    setPassword(newPassword);
    return newPassword;
  };

  const updatePassword = (newPassword: string) => {
    accessPasswordService.savePassword(newPassword);
    setPassword(newPassword);
  };

  const setRefreshModeAndSave = (mode: RefreshMode) => {
    accessPasswordService.setRefreshMode(mode);
    setRefreshMode(mode);
    // 如果设置为单次，立即刷新一次
    if (mode === "once") {
      onceRefreshedRef.current = false;
    }
  };

  return {
    password,
    loading,
    refreshing,
    refreshMode,
    generateNew,
    updatePassword,
    refreshPassword,
    setRefreshMode: setRefreshModeAndSave,
  };
}
