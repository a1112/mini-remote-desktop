import { useCallback, useEffect, useMemo, useState } from "react";
import { Laptop, Monitor, Server, Smartphone } from "lucide-react";
import {
  ipcLanDiscoverySnapshot,
  ipcRefreshLanDiscovery,
  type LanPeerInfo,
} from "../adapters/tauri";
import { deviceService } from "../services/deviceService";
import { isTauriRuntime } from "../utils/runtime";
import { useAuth } from "./AuthContext";

export type DeviceDiscoverySource = "local" | "lan_p2p" | "server";

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
  discoverySources: DeviceDiscoverySource[];
  primarySource: DeviceDiscoverySource;
  sourceLabel: string;
  isLocal: boolean;
  p2pAvailable: boolean;
  serverAvailable: boolean;
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

type StoredDeviceInfo = NonNullable<ReturnType<typeof deviceService.getDeviceInfo>>;

const iconMap: Record<string, typeof Monitor> = {
  Monitor,
  Laptop,
  Server,
  Smartphone,
};

const sourceLabels: Record<DeviceDiscoverySource, string> = {
  local: "本机",
  lan_p2p: "P2P 局域网",
  server: "服务器",
};

const API_BASE =
  (import.meta as any).env?.VITE_RDESK_SERVER_URL ?? "http://127.0.0.1:9530/api/v1";

const uniqueSources = (sources: DeviceDiscoverySource[]) =>
  Array.from(new Set(sources));

const sourceLabel = (sources: DeviceDiscoverySource[]) =>
  uniqueSources(sources).map((source) => sourceLabels[source]).join(" / ");

const localPlatformLabel = () => {
  if (typeof navigator === "undefined") return "Local Rdesk node";
  return (navigator as any).userAgentData?.platform ?? navigator.platform ?? "Local Rdesk node";
};

const baseDevice = (
  source: DeviceDiscoverySource,
  fields: Omit<
    Device,
    | "discoverySources"
    | "primarySource"
    | "sourceLabel"
    | "isLocal"
    | "p2pAvailable"
    | "serverAvailable"
  >
): Device => {
  const discoverySources = [source];
  return {
    ...fields,
    discoverySources,
    primarySource: source,
    sourceLabel: sourceLabel(discoverySources),
    isLocal: source === "local",
    p2pAvailable: source === "lan_p2p",
    serverAvailable: source === "server",
  };
};

const toServerDevice = (item: DeviceApi): Device =>
  baseDevice("server", {
    id: item.id || item.device_id,
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
  if (ageMs < 1000) return "刚刚";
  const seconds = Math.round(ageMs / 1000);
  if (seconds < 60) return `${seconds} 秒前`;
  return `${Math.round(seconds / 60)} 分钟前`;
};

const lanPeerToDevice = (peer: LanPeerInfo): Device =>
  baseDevice("lan_p2p", {
    id: peer.device_id,
    name: peer.device_name,
    deviceId: peer.device_id,
    os: `${peer.device_type || "Rdesk LAN"} / ${peer.transports.join(", ") || "direct"}`,
    icon: Monitor,
    status: peer.p2p_available ? "online" : "offline",
    location: `P2P ${peer.p2p_control_addr}`,
    ping: peer.age_ms < 5000 ? Math.max(1, Math.round(peer.age_ms / 100)) : null,
    lastSeen: formatLanLastSeen(peer.age_ms),
    cpu: null,
    ram: null,
    disk: null,
    ip: peer.ip,
    group: "LAN P2P",
    favorite: false,
  });

const localDeviceToDevice = (info: StoredDeviceInfo): Device =>
  baseDevice("local", {
    id: info.device_id,
    name: info.device_name || "This device",
    deviceId: info.device_id,
    os: localPlatformLabel(),
    icon: Monitor,
    status: "online",
    location: "This device",
    ping: 0,
    lastSeen: "本机在线",
    cpu: null,
    ram: null,
    disk: null,
    ip: "127.0.0.1",
    group: "Local",
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

async function fetchLocalDevice(): Promise<Device | null> {
  const info = deviceService.getDeviceInfo() ?? (await deviceService.initialize());
  return info ? localDeviceToDevice(info) : null;
}

function mergeDevice(existing: Device, incoming: Device): Device {
  const discoverySources = uniqueSources([
    ...existing.discoverySources,
    ...incoming.discoverySources,
  ]);
  const isLocal = existing.isLocal || incoming.isLocal;
  const p2pAvailable = existing.p2pAvailable || incoming.p2pAvailable;
  const serverAvailable = existing.serverAvailable || incoming.serverAvailable;
  const status =
    existing.status === "online" || incoming.status === "online" ? "online" : "offline";
  const serverSide = existing.serverAvailable ? existing : incoming.serverAvailable ? incoming : null;
  const p2pSide = incoming.p2pAvailable ? incoming : existing.p2pAvailable ? existing : null;
  const localSide = incoming.isLocal ? incoming : existing.isLocal ? existing : null;

  return {
    ...existing,
    id: serverSide?.id ?? localSide?.id ?? p2pSide?.id ?? existing.id,
    name: serverSide?.name ?? localSide?.name ?? p2pSide?.name ?? incoming.name,
    os: serverSide?.os ?? localSide?.os ?? p2pSide?.os ?? incoming.os,
    icon: serverSide?.icon ?? localSide?.icon ?? p2pSide?.icon ?? incoming.icon,
    status,
    location: p2pSide?.location ?? localSide?.location ?? serverSide?.location ?? incoming.location,
    ping: p2pSide?.ping ?? serverSide?.ping ?? localSide?.ping ?? incoming.ping,
    lastSeen: localSide?.lastSeen ?? p2pSide?.lastSeen ?? serverSide?.lastSeen ?? incoming.lastSeen,
    cpu: serverSide?.cpu ?? existing.cpu ?? incoming.cpu,
    ram: serverSide?.ram ?? existing.ram ?? incoming.ram,
    disk: serverSide?.disk ?? existing.disk ?? incoming.disk,
    ip: p2pSide?.ip ?? serverSide?.ip ?? localSide?.ip ?? incoming.ip,
    group: localSide?.group ?? p2pSide?.group ?? serverSide?.group ?? incoming.group,
    favorite: existing.favorite || incoming.favorite,
    discoverySources,
    primarySource: isLocal ? "local" : p2pAvailable ? "lan_p2p" : "server",
    sourceLabel: sourceLabel(discoverySources),
    isLocal,
    p2pAvailable,
    serverAvailable,
  };
}

function mergeDevices(
  serverDevices: Device[],
  lanDevices: Device[],
  localDevice: Device | null
): Device[] {
  const byDeviceId = new Map<string, Device>();
  const add = (device: Device) => {
    const existing = byDeviceId.get(device.deviceId);
    byDeviceId.set(device.deviceId, existing ? mergeDevice(existing, device) : device);
  };

  serverDevices.forEach(add);
  lanDevices.forEach(add);
  if (localDevice) add(localDevice);

  return Array.from(byDeviceId.values()).sort((a, b) => {
    if (a.isLocal !== b.isLocal) return a.isLocal ? -1 : 1;
    if (a.status !== b.status) return a.status === "online" ? -1 : 1;
    if (a.p2pAvailable !== b.p2pAvailable) return a.p2pAvailable ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
}

export interface UseDevicesOptions {
  pollInterval?: number;
  enabled?: boolean;
}

export function useDevices(options?: UseDevicesOptions) {
  const [devices, setDevices] = useState<Device[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const [currentDeviceId, setCurrentDeviceId] = useState<string | null>(null);
  const { isLoggedIn, token } = useAuth();

  const { pollInterval = 30000, enabled = true } = options || {};

  const fetchDevices = useCallback(async () => {
    if (devices.length === 0) setLoading(true);
    setError(null);

    const [lanDevices, localDevice] = await Promise.all([
      fetchLanDevices(true),
      fetchLocalDevice(),
    ]);
    setCurrentDeviceId(localDevice?.deviceId ?? deviceService.getDeviceId());

    if (!isLoggedIn || !token) {
      setDevices(mergeDevices([], lanDevices, localDevice));
      setLoading(false);
      setLastUpdated(new Date());
      return;
    }

    try {
      const resp = await fetch(`${API_BASE}/devices`, {
        headers: {
          Authorization: `Bearer ${token}`,
        },
      });
      if (!resp.ok) throw new Error(`Request failed: ${resp.status}`);
      const data = (await resp.json()) as DeviceApi[];
      setDevices(mergeDevices(data.map(toServerDevice), lanDevices, localDevice));
      setLastUpdated(new Date());
    } catch (e) {
      setDevices(mergeDevices([], lanDevices, localDevice));
      setLastUpdated(new Date());
      setError(
        lanDevices.length > 0 || localDevice
          ? null
          : e instanceof Error
            ? e.message
            : "加载设备失败"
      );
    } finally {
      setLoading(false);
    }
  }, [devices.length, isLoggedIn, token]);

  useEffect(() => {
    void fetchDevices();
  }, [fetchDevices]);

  useEffect(() => {
    if (!enabled) return;
    const interval = setInterval(() => {
      if (!document.hidden) void fetchDevices();
    }, pollInterval);
    return () => clearInterval(interval);
  }, [fetchDevices, pollInterval, enabled]);

  useEffect(() => {
    if (!enabled) return;
    const handleVisibilityChange = () => {
      if (!document.hidden) void fetchDevices();
    };
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => document.removeEventListener("visibilitychange", handleVisibilityChange);
  }, [fetchDevices, enabled]);

  return { devices, loading, error, lastUpdated, refresh: fetchDevices, currentDeviceId };
}

export function useDeviceById(id: string | undefined, devices: Device[]) {
  return useMemo(() => devices.find((d) => d.id === id), [id, devices]);
}
