import { useCallback, useEffect, useMemo, useState } from "react";
import { Laptop, Monitor, Server, Smartphone } from "lucide-react";
import { ipcLanDiscoverySnapshot, ipcRefreshLanDiscovery, type LanPeerInfo } from "../adapters/tauri";
import { deviceService } from "../services/deviceService";
import { isTauriRuntime } from "../utils/runtime";
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

const formatLanLastSeen = (ageMs: number) => {
  if (ageMs < 1000) return "just now";
  const seconds = Math.round(ageMs / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  return `${Math.round(seconds / 60)}m ago`;
};

const lanPeerToDevice = (peer: LanPeerInfo): Device => ({
  id: peer.device_id,
  name: peer.device_name,
  deviceId: peer.device_id,
  os: `LAN P2P / ${peer.transports.join(", ") || "direct"}`,
  icon: Monitor,
  status: peer.p2p_available ? "online" : "offline",
  location: `LAN ${peer.p2p_control_addr}`,
  ping: peer.age_ms < 5000 ? Math.max(1, Math.round(peer.age_ms / 100)) : null,
  lastSeen: formatLanLastSeen(peer.age_ms),
  cpu: null,
  ram: null,
  disk: null,
  ip: peer.ip,
  group: "LAN",
  favorite: false,
});

async function fetchLanDevices(triggerProbe: boolean): Promise<Device[]> {
  if (!isTauriRuntime()) return [];
  const result = triggerProbe
    ? await ipcRefreshLanDiscovery()
    : await ipcLanDiscoverySnapshot();
  if (!result.ok) return [];
  return result.value.peers.map(lanPeerToDevice);
}

function mergeDevices(serverDevices: Device[], lanDevices: Device[]): Device[] {
  const byDeviceId = new Map<string, Device>();
  for (const device of serverDevices) {
    byDeviceId.set(device.deviceId, device);
  }
  for (const device of lanDevices) {
    const existing = byDeviceId.get(device.deviceId);
    byDeviceId.set(
      device.deviceId,
      existing ? { ...existing, ...device, favorite: existing.favorite } : device
    );
  }
  return Array.from(byDeviceId.values());
}

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
    const lanDevices = await fetchLanDevices(true);
    // 只有登录后才加载设备列表
    if (!isLoggedIn || !token) {
      setDevices(lanDevices);
      setLoading(false);
      setError(null);
      setLastUpdated(new Date());
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
      setDevices(mergeDevices(data.map(toDevice), lanDevices));
      setLastUpdated(new Date());
    } catch (e) {
      if (lanDevices.length > 0) {
        setDevices(lanDevices);
        setLastUpdated(new Date());
        setError(null);
        return;
      }
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
      void fetchLanDevices(false).then((lanDevices) => {
        setDevices(lanDevices);
        setLastUpdated(new Date());
        setLoading(false);
      });
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
