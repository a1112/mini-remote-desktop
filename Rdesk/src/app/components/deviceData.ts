import { useCallback, useEffect, useMemo, useState } from "react";
import { Laptop, Monitor, Server, Smartphone } from "lucide-react";
import { deviceService } from "../services/deviceService";
import { useAuth } from "./AuthContext";

export interface Device {
  id: string;
  name: string;
  deviceId: string;
  os: string;
  icon: typeof Monitor;
  status: "online" | "offline";
  location: string;
  ping: number | null;
  lastSeen: string;
  cpu: number | null;
  ram: number | null;
  disk: number | null;
  ip: string;
  group: string;
  favorite: boolean;
}

type DeviceApi = {
  id: string;
  name: string;
  device_id: string;
  os: string;
  icon: "Monitor" | "Laptop" | "Server" | "Smartphone" | string;
  status: "online" | "offline";
  location: string;
  ping: number | null;
  last_seen: string;
  cpu: number | null;
  ram: number | null;
  disk: number | null;
  ip: string;
  group: string;
  favorite: boolean;
};

const iconMap: Record<string, typeof Monitor> = {
  Monitor,
  Laptop,
  Server,
  Smartphone,
};

const API_BASE =
  (import.meta as any).env?.VITE_RDESK_SERVER_URL ?? "http://127.0.0.1:9530/api/v1";

const toDevice = (item: DeviceApi): Device => ({
  id: item.id,
  name: item.name,
  deviceId: item.device_id,
  os: item.os,
  icon: iconMap[item.icon] ?? Monitor,
  status: item.status,
  location: item.location,
  ping: item.ping,
  lastSeen: item.last_seen,
  cpu: item.cpu,
  ram: item.ram,
  disk: item.disk,
  ip: item.ip,
  group: item.group,
  favorite: item.favorite,
});

export interface UseDevicesOptions {
  pollInterval?: number;      // 轮询间隔（毫秒），默认 30000
  enabled?: boolean;          // 是否启用轮询，默认 true
}

export function useDevices(options?: UseDevicesOptions) {
  const [devices, setDevices] = useState<Device[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const [currentDeviceId, setCurrentDeviceId] = useState<string | null>(null);
  const { isLoggedIn, token } = useAuth();

  const {
    pollInterval = 30000,
    enabled = true,
  } = options || {};

  const fetchDevices = useCallback(async () => {
    // 只有登录后才加载设备列表
    if (!isLoggedIn || !token) {
      setDevices([]);
      setLoading(false);
      setError(null);
      return;
    }

    setLoading(true);
    setError(null);
    try {
      const resp = await fetch(`${API_BASE}/devices`, {
        headers: {
          "Authorization": `Bearer ${token}`,
        },
      });
      if (!resp.ok) throw new Error(`Request failed: ${resp.status}`);
      const data = (await resp.json()) as DeviceApi[];
      setDevices(data.map(toDevice));
      setLastUpdated(new Date());
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载设备失败");
    } finally {
      setLoading(false);
    }
  }, [isLoggedIn, token]);

  // 初始加载
  useEffect(() => {
    // 获取当前设备 ID
    const deviceId = deviceService.getDeviceId();
    if (deviceId) {
      setCurrentDeviceId(deviceId);
    }
    // 只有登录后才加载设备列表
    if (isLoggedIn && token) {
      fetchDevices();
    } else {
      setDevices([]);
      setLoading(false);
    }
  }, [fetchDevices, isLoggedIn, token]);

  // 轮询
  useEffect(() => {
    if (!enabled) return;
    const interval = setInterval(() => {
      if (!document.hidden) fetchDevices();
    }, pollInterval);
    return () => clearInterval(interval);
  }, [fetchDevices, pollInterval, enabled]);

  // 页面恢复时刷新
  useEffect(() => {
    if (!enabled) return;
    const handleVisibilityChange = () => {
      if (!document.hidden) fetchDevices();
    };
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => document.removeEventListener("visibilitychange", handleVisibilityChange);
  }, [fetchDevices, enabled]);

  return { devices, loading, error, lastUpdated, refresh: fetchDevices, currentDeviceId };
}

export function useDeviceById(id: string | undefined, devices: Device[]) {
  return useMemo(() => devices.find((d) => d.id === id), [id, devices]);
}
