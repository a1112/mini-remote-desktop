import { useDevices } from "./deviceData";
import { AppVersionBadge } from "./AppVersionBadge";
import { useState, useEffect, useRef, useCallback } from "react";
import { deviceService } from "../services/deviceService";
import { deviceActionService } from "../services/deviceActionService";
import {
  listSessions,
  stopSession,
  type SessionInfo,
} from "../services/ipcSessionService";
import { ipcCapabilitySnapshot, ipcPeerCapabilitySnapshot } from "../adapters/tauri";
import { useTheme } from "./ThemeContext";
import { isTauriRuntime } from "../utils/runtime";
import { useAuth } from "./AuthContext";
import { NavLink, useLocation, useNavigate } from "react-router";
import {
  Monitor,
  MonitorUp,
  Laptop,
  History,
  Settings,
  Server,
  Smartphone,
  ChevronDown,
  MoreHorizontal,
  Play,
  FolderOpen as FolderIcon,
  Terminal,
  Pencil,
  Power,
  Trash2,
  Copy,
  Star,
  RefreshCw,
  LogOut,
  Check,
  X,
  Ban,
  RotateCw,
  Zap,
  Info,
} from "lucide-react";

interface SidebarProps {
  collapsed: boolean;
}

const navItems = [
  { to: "/", label: "远程控制", icon: MonitorUp, end: true },
  { to: "/devices", label: "我的设备", icon: Monitor },
  { to: "/connections", label: "连接记录", icon: History },
];

const iconMap: Record<string, typeof Monitor> = {
  Monitor,
  Terminal,
  Laptop,
  Server,
  Smartphone,
};

type DeviceActionStatus = {
  kind: "success" | "error";
  message: string;
};

type DeviceMenuItem = {
  icon?: typeof Monitor;
  label?: string;
  action?: () => void | Promise<void>;
  type?: "divider";
  danger?: boolean;
  submenu?: "management";
  disabled?: boolean;
  title?: string;
};

const unsupportedDeviceActionTitle = "暂未接入本机服务能力";
const terminalSessionStates = new Set(["failed", "closed"]);
const runnableCapabilityStatuses = new Set(["available", "usable", "supported", "degraded"]);

type CapabilitySnapshotLike = {
  capabilities?: Array<{
    id?: string;
    status?: string;
    reason?: string | null;
    detail?: string | null;
  }>;
};

function activePeerSessionForDevice(
  sessions: SessionInfo[],
  deviceId: string
): SessionInfo | null {
  return sessions.find(
    (session) =>
      session.peer_device_id === deviceId &&
      !terminalSessionStates.has(session.state)
  ) ?? null;
}

function remotePowerUnavailableReasonFromSnapshot(
  snapshot: CapabilitySnapshotLike
): string | null {
  const capability = snapshot.capabilities?.find(
    (item) => item.id === "control.remote_power"
  );
  if (!capability) return null;
  const status = capability.status?.trim().toLowerCase();
  if (status && runnableCapabilityStatuses.has(status)) return null;
  return (
    capability.reason?.trim() ||
    capability.detail?.trim() ||
    "远程重启/关机能力未启用"
  );
}

export function Sidebar({ collapsed }: SidebarProps) {
  const native = isTauriRuntime();
  const { devices, refresh, currentDeviceId } = useDevices({ pollInterval: 30000, enabled: true });
  const [devicesExpanded, setDevicesExpanded] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [submenuOpen, setSubmenuOpen] = useState<string | null>(null);
  const [actionStatus, setActionStatus] = useState<DeviceActionStatus | null>(null);
  const [sessionSummaries, setSessionSummaries] = useState<SessionInfo[]>([]);
  const [remotePowerUnavailableReason, setRemotePowerUnavailableReason] = useState<string | null>(null);
  const [peerRemotePowerUnavailableReason, setPeerRemotePowerUnavailableReason] = useState<string | null>(null);
  const actionStatusTimerRef = useRef<number | null>(null);
  const { isLoggedIn, user } = useAuth();

  const refreshSessionSummaries = useCallback(async () => {
    if (!native) return;
    try {
      setSessionSummaries(await listSessions());
    } catch {
      setSessionSummaries([]);
    }
  }, [native]);

  const handleRefresh = async () => {
    setRefreshing(true);
    await Promise.all([refresh(), refreshSessionSummaries()]);
    setTimeout(() => setRefreshing(false), 500);
  };
  const [contextMenu, setContextMenu] = useState<{ deviceId: string; x: number; y: number } | null>(null);
  const [editingDeviceId, setEditingDeviceId] = useState<string | null>(null);
  const [editingName, setEditingName] = useState("");
  const [renameSaved, setRenameSaved] = useState(false);
  const contextMenuRef = useRef<HTMLDivElement>(null);
  const editInputRef = useRef<HTMLInputElement>(null);
  const location = useLocation();
  const navigate = useNavigate();
  const { isDark } = useTheme();

  const onlineDevices = devices.filter((d) => d.status === "online");
  const offlineDevices = devices.filter((d) => d.status === "offline");

  const showActionStatus = (next: DeviceActionStatus) => {
    setActionStatus(next);
    if (actionStatusTimerRef.current !== null) {
      window.clearTimeout(actionStatusTimerRef.current);
    }
    actionStatusTimerRef.current = window.setTimeout(() => {
      setActionStatus(null);
      actionStatusTimerRef.current = null;
    }, 3000);
  };

  useEffect(() => () => {
    if (actionStatusTimerRef.current !== null) {
      window.clearTimeout(actionStatusTimerRef.current);
    }
  }, []);

  useEffect(() => {
    void refreshSessionSummaries();
  }, [refreshSessionSummaries]);

  useEffect(() => {
    if (!native) return;
    let cancelled = false;
    void ipcCapabilitySnapshot()
      .then((result) => {
        if (cancelled) return;
        if (!result.ok) {
          setRemotePowerUnavailableReason(null);
          return;
        }
        setRemotePowerUnavailableReason(
          remotePowerUnavailableReasonFromSnapshot(result.value)
        );
      })
      .catch(() => {
        if (!cancelled) setRemotePowerUnavailableReason(null);
      });
    return () => {
      cancelled = true;
    };
  }, [native]);

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (contextMenuRef.current && !contextMenuRef.current.contains(e.target as Node)) {
        setContextMenu(null);
      }
    };
    if (contextMenu) {
      document.addEventListener("mousedown", handleClickOutside);
      return () => document.removeEventListener("mousedown", handleClickOutside);
    }
  }, [contextMenu]);

  // 自动聚焦编辑输入框
  useEffect(() => {
    if (editingDeviceId) {
      editInputRef.current?.focus();
      editInputRef.current?.select();
    }
  }, [editingDeviceId]);

  const handleContextMenu = (e: React.MouseEvent, deviceId: string) => {
    e.preventDefault();
    e.stopPropagation();
    setContextMenu({ deviceId, x: e.clientX, y: e.clientY });
  };

  // 阻止设备列表区域的浏览器默认右键菜单
  const handleListContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
  };

  const handleMoreClick = (e: React.MouseEvent, deviceId: string) => {
    e.stopPropagation();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    setContextMenu({ deviceId, x: rect.right, y: rect.bottom });
  };

  const contextMenuDevice = contextMenu ? devices.find((d) => d.id === contextMenu.deviceId) : null;
  const isContextOnline = contextMenuDevice?.status === "online";
  const isContextDisabled = Boolean(contextMenuDevice?.disabled);
  const isContextRemoteActionAvailable =
    Boolean(contextMenuDevice) &&
    isContextOnline &&
    !isContextDisabled &&
    !contextMenuDevice?.isLocal;
  const contextActiveSession = contextMenuDevice
    ? activePeerSessionForDevice(sessionSummaries, contextMenuDevice.deviceId)
    : null;

  useEffect(() => {
    let cancelled = false;
    setPeerRemotePowerUnavailableReason(null);
    if (!native || !contextMenuDevice || contextMenuDevice.isLocal) {
      return () => {
        cancelled = true;
      };
    }

    void ipcPeerCapabilitySnapshot(contextMenuDevice.deviceId)
      .then((result) => {
        if (cancelled) return;
        if (!result.ok || !result.value) {
          setPeerRemotePowerUnavailableReason(null);
          return;
        }
        setPeerRemotePowerUnavailableReason(
          remotePowerUnavailableReasonFromSnapshot(result.value)
        );
      })
      .catch(() => {
        if (!cancelled) setPeerRemotePowerUnavailableReason(null);
      });

    return () => {
      cancelled = true;
    };
  }, [native, contextMenuDevice?.deviceId, contextMenuDevice?.isLocal]);

  // 重命名设备
  const handleStartRename = (deviceId: string, currentName: string) => {
    setEditingDeviceId(deviceId);
    setEditingName(currentName);
    setContextMenu(null);
    setTimeout(() => editInputRef.current?.focus(), 0);
  };

  const handleConfirmRename = async () => {
    if (editingName.trim() && editingDeviceId) {
      // 调用 API 更新设备名称
      const success = await deviceService.renameDevice(editingDeviceId, editingName.trim());
      if (success) {
        // 刷新设备列表
        await refresh();
        setRenameSaved(true);
        setTimeout(() => setRenameSaved(false), 2000);
      }
    }
    setEditingDeviceId(null);
  };

  const handleCancelRename = () => {
    setEditingDeviceId(null);
    setEditingName("");
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      handleConfirmRename();
    } else if (e.key === "Escape") {
      e.preventDefault();
      handleCancelRename();
    }
  };

  // 复制设备 ID
  const handleCopyId = (deviceId: string) => {
    navigator.clipboard.writeText(deviceId);
    setContextMenu(null);
  };

  const handleUnbind = async (deviceId: string, deviceName: string) => {
    setContextMenu(null);
    setSubmenuOpen(null);

    if (!isLoggedIn || !user) {
      showActionStatus({ kind: "error", message: "请先登录后再退出绑定" });
      return;
    }

    try {
      const success = await deviceService.unbindDevice(user.id, deviceId);
      if (!success) {
        showActionStatus({ kind: "error", message: `退出绑定失败：${deviceName}` });
        return;
      }
      await refresh();
      showActionStatus({ kind: "success", message: `已退出绑定：${deviceName}` });
    } catch (error) {
      const message = error instanceof Error ? error.message : "未知错误";
      showActionStatus({ kind: "error", message: `退出绑定失败：${message}` });
    }
  };

  const handleOpenDeviceTab = (deviceId: string, tab: "remote" | "files" | "apps" | "info" | "terminal") => {
    navigate(`/devices/${deviceId}?tab=${tab}`);
    setContextMenu(null);
    setSubmenuOpen(null);
  };

  const handleToggleFavorite = async (deviceId: string, deviceName: string, favorite: boolean) => {
    await deviceActionService.setDeviceFavorite(deviceId, favorite);
    setContextMenu(null);
    setSubmenuOpen(null);
    await refresh();
    showActionStatus({
      kind: "success",
      message: favorite ? `已收藏：${deviceName}` : `已取消收藏：${deviceName}`,
    });
  };

  const handleRemoveDevice = async (deviceId: string, deviceName: string, isLocal: boolean) => {
    setContextMenu(null);
    setSubmenuOpen(null);
    if (isLocal) {
      showActionStatus({ kind: "error", message: "不能移除本机设备" });
      return;
    }
    if (
      typeof window.confirm === "function" &&
      !window.confirm(`移除设备“${deviceName}”？可通过重新发现或重新登录恢复。`)
    ) {
      return;
    }
    await deviceActionService.markDeviceRemoved(deviceId);
    await refresh();
    showActionStatus({ kind: "success", message: `已移除：${deviceName}` });
  };

  const handleToggleDisabled = async (
    deviceId: string,
    deviceName: string,
    disabled: boolean,
    isLocal: boolean
  ) => {
    setContextMenu(null);
    setSubmenuOpen(null);
    if (isLocal) {
      showActionStatus({ kind: "error", message: "不能禁用本机设备" });
      return;
    }
    await deviceActionService.setDeviceDisabled(deviceId, disabled);
    await refresh();
    showActionStatus({
      kind: "success",
      message: disabled ? `已禁用：${deviceName}` : `已解除禁用：${deviceName}`,
    });
  };

  const handleDisconnectDevice = async (sessionId: string, deviceName: string) => {
    setContextMenu(null);
    setSubmenuOpen(null);
    try {
      await stopSession(sessionId);
      await Promise.all([refresh(), refreshSessionSummaries()]);
      showActionStatus({ kind: "success", message: `已断开连接：${deviceName}` });
    } catch (error) {
      const message = error instanceof Error ? error.message : "未知错误";
      showActionStatus({ kind: "error", message: `断开连接失败：${message}` });
    }
  };

  const handleWakeOnLan = async (
    deviceId: string,
    deviceName: string,
    macAddress: string
  ) => {
    setContextMenu(null);
    setSubmenuOpen(null);
    try {
      await deviceActionService.wakeOnLan({
        deviceId,
        macAddress,
        broadcastAddr: undefined,
      });
      showActionStatus({ kind: "success", message: `已发送唤醒包：${deviceName}` });
    } catch (error) {
      const message = error instanceof Error ? error.message : "未知错误";
      showActionStatus({ kind: "error", message: `唤醒失败：${message}` });
    }
  };

  const handleRemotePowerAction = async (
    deviceId: string,
    deviceName: string,
    action: "restart" | "shutdown"
  ) => {
    setContextMenu(null);
    setSubmenuOpen(null);
    const actionLabel = action === "restart" ? "重启" : "关机";
    try {
      await deviceActionService.requestRemoteDevicePowerAction({
        deviceId,
        action,
      });
      showActionStatus({ kind: "success", message: `已请求远端${actionLabel}：${deviceName}` });
    } catch (error) {
      const message = error instanceof Error ? error.message : "未知错误";
      showActionStatus({ kind: "error", message: `远端${actionLabel}失败：${message}` });
    }
  };

  // 菜单项定义（用于非二级菜单渲染）
  const getTopLevelMenuItems = () => {
    const items: DeviceMenuItem[] = [];
    const activeContextMenu = contextMenu;

    if (!activeContextMenu) {
      return items;
    }

    // 在线设备特有菜单
    if (isContextOnline && !isContextDisabled) {
      items.push(
        { icon: Play, label: "远程桌面", action: () => { navigate(`/devices/${activeContextMenu.deviceId}`); setContextMenu(null); } },
        {
          icon: FolderIcon,
          label: "文件传输",
          action: () => handleOpenDeviceTab(activeContextMenu.deviceId, "files"),
        },
        {
          icon: Terminal,
          label: "远程终端",
          action: () => handleOpenDeviceTab(activeContextMenu.deviceId, "terminal"),
        },
        { type: "divider" as const }
      );
    }

    // 通用菜单
    items.push(
      {
        icon: Pencil,
        label: "重命名",
        action: () => {
          if (contextMenuDevice) {
            handleStartRename(activeContextMenu.deviceId, contextMenuDevice.name);
          }
        },
      },
      {
        icon: Star,
        label: contextMenuDevice?.favorite ? "取消收藏" : "收藏设备",
        action: () => {
          if (contextMenuDevice) {
            return handleToggleFavorite(
              contextMenuDevice.deviceId,
              contextMenuDevice.name,
              !contextMenuDevice.favorite
            );
          }
        },
      },
      {
        icon: Copy,
        label: "复制 ID",
        action: () => {
          if (contextMenuDevice) {
            handleCopyId(contextMenuDevice.deviceId);
          }
        },
      },
      { type: "divider" as const }
    );

    // 启用/禁用
    items.push({
      icon: Ban,
      label: contextMenuDevice?.disabled ? "解除禁用" : "禁用设备",
      action: () => {
        if (contextMenuDevice) {
          return handleToggleDisabled(
            contextMenuDevice.deviceId,
            contextMenuDevice.name,
            !contextMenuDevice.disabled,
            contextMenuDevice.isLocal
          );
        }
      },
      disabled: contextMenuDevice?.isLocal,
      title: contextMenuDevice?.isLocal ? "不能禁用本机设备" : undefined,
    });

    items.push({ type: "divider" as const });

    // 在线设备特有菜单
    if (isContextOnline && !isContextDisabled) {
      items.push(
        {
          icon: LogOut,
          label: "退出绑定",
          action: () => {
            if (contextMenuDevice) {
              return handleUnbind(contextMenuDevice.deviceId, contextMenuDevice.name);
            }
          },
          disabled: !isLoggedIn || !user,
          title: !isLoggedIn || !user ? "请先登录后再退出绑定" : undefined,
        },
        {
          icon: Power,
          label: "断开连接",
          action: () => {
            if (contextMenuDevice && contextActiveSession) {
              return handleDisconnectDevice(
                contextActiveSession.session_id,
                contextMenuDevice.name
              );
            }
          },
          disabled: !contextActiveSession || !isContextRemoteActionAvailable,
          title: !isContextRemoteActionAvailable
            ? isContextDisabled
              ? "设备已禁用"
              : contextMenuDevice?.isLocal
                ? "不能断开本机设备"
                : "设备离线"
            : contextActiveSession
              ? undefined
              : "没有可断开的活跃会话",
          danger: true,
        }
      );
    }

    // 移除设备和管理子菜单
    items.push(
      {
        icon: Trash2,
        label: "移除设备",
        action: () => {
          if (contextMenuDevice) {
            return handleRemoveDevice(
              contextMenuDevice.deviceId,
              contextMenuDevice.name,
              contextMenuDevice.isLocal
            );
          }
        },
        disabled: contextMenuDevice?.isLocal,
        title: contextMenuDevice?.isLocal ? "不能移除本机设备" : undefined,
        danger: true,
      },
      { icon: Settings, label: "管理", submenu: "management" }
    );

    return items;
  };

  // 管理子菜单项
  const getManagementSubmenuItems = () => {
    const canWakeOnLan =
      Boolean(contextMenuDevice?.macAddress) &&
      contextMenuDevice?.status === "offline" &&
      !contextMenuDevice?.disabled &&
      !contextMenuDevice?.isLocal;
    const wakeDisabledReason = contextMenuDevice?.isLocal
      ? "不能唤醒本机设备"
      : contextMenuDevice?.disabled
        ? "设备已禁用"
      : contextMenuDevice?.status === "online"
        ? "设备当前在线"
        : contextMenuDevice?.macAddress
          ? undefined
          : "缺少设备 MAC 地址";
    const remotePowerDisabledReason = isContextDisabled
      ? "设备已禁用"
      : !isContextOnline
        ? "设备离线"
        : contextMenuDevice?.isLocal
          ? "不能远端控制本机电源"
          : peerRemotePowerUnavailableReason ?? remotePowerUnavailableReason ?? undefined;
    const remotePowerDisabled =
      !isContextRemoteActionAvailable ||
      Boolean(peerRemotePowerUnavailableReason ?? remotePowerUnavailableReason);

    return [
      {
        icon: RotateCw,
        label: "重启",
        disabled: remotePowerDisabled,
        title: remotePowerDisabledReason,
        action: () => {
          if (contextMenuDevice) {
            return handleRemotePowerAction(
              contextMenuDevice.deviceId,
              contextMenuDevice.name,
              "restart"
            );
          }
        },
      },
      {
        icon: Power,
        label: "关机",
        disabled: remotePowerDisabled,
        title: remotePowerDisabledReason,
        action: () => {
          if (contextMenuDevice) {
            return handleRemotePowerAction(
              contextMenuDevice.deviceId,
              contextMenuDevice.name,
              "shutdown"
            );
          }
        },
      },
      {
        icon: Zap,
        label: "Wake-on-LAN",
        disabled: !canWakeOnLan,
        title: wakeDisabledReason,
        action: () => {
          if (contextMenuDevice?.macAddress) {
            return handleWakeOnLan(
              contextMenuDevice.deviceId,
              contextMenuDevice.name,
              contextMenuDevice.macAddress
            );
          }
        },
      },
      {
        icon: Info,
        label: "设备信息",
        action: () => {
          if (contextMenuDevice) {
            handleOpenDeviceTab(contextMenuDevice.deviceId, "info");
          }
        },
      },
    ];
  };

  return (
    <aside
      className={`remote-sidebar relative flex shrink-0 flex-col border-r transition-[width] duration-200 ${collapsed ? "remote-sidebar-collapsed" : ""}`}
      style={{ width: collapsed ? 54 : 196 }}
    >
      <div className="remote-brand" aria-label="Rdesk">
        <span className="remote-brand-mark"><MonitorUp size={18} aria-hidden="true" /></span>
        {!collapsed && <span>Rdesk</span>}
      </div>

      <nav aria-label="主导航" className="shrink-0 space-y-1 px-2 pb-3">
        {navItems.map(({ to, label, icon: Icon, end }) => (
          <NavLink key={to} to={to} end={end} className="remote-nav-link" aria-label={label} title={collapsed ? label : undefined}>
            <Icon size={17} className="shrink-0" aria-hidden="true" />
            {!collapsed && <span>{label}</span>}
          </NavLink>
        ))}
      </nav>

      {/* Device list section */}
      {!collapsed && (
        <div className={`flex-1 overflow-y-auto border-t ${isDark ? "border-gray-700" : "border-gray-300/30"}`}>
          {/* Section header */}
          <div className="flex items-center justify-between px-3 py-2">
            <button
              onClick={() => setDevicesExpanded(!devicesExpanded)}
              aria-expanded={devicesExpanded}
              className={`flex items-center gap-1.5 transition-colors ${isDark ? "text-gray-400 hover:text-gray-200" : "text-gray-500 hover:text-gray-700"}`}
            >
              <ChevronDown
                className={`transition-transform ${devicesExpanded ? "" : "-rotate-90"}`}
                style={{ width: 12, height: 12 }}
              />
              <span style={{ fontSize: 11 }} className="uppercase tracking-wider font-medium">
                设备列表
              </span>
            </button>
            <div className="flex items-center gap-1">
              <button
                onClick={handleRefresh}
                className={`p-0.5 rounded transition-colors ${
                  isDark ? "text-gray-400 hover:bg-gray-700 hover:text-gray-200" : "text-gray-400 hover:bg-gray-100 hover:text-gray-600"
                } ${refreshing ? "animate-spin" : ""}`}
                title="刷新设备列表"
              >
                <RefreshCw style={{ width: 14, height: 14 }} />
              </button>
            </div>
          </div>

          {devicesExpanded && (
            <div className="px-1.5 pb-2" onContextMenu={handleListContextMenu}>
              {devices.length === 0 && <p className="px-2 py-1 text-xs text-[color:var(--desk-muted)]">暂无设备</p>}
              {/* Online devices */}
              {onlineDevices.map((device) => {
                const isActive = location.pathname === `/devices/${device.id}`;
                const DeviceIcon = device.icon;
                const isCurrentDevice = device.deviceId === currentDeviceId;
                const isEditing = editingDeviceId === device.id;
                return (
                  <div
                    key={device.id}
                    onContextMenu={(e) => !isEditing && handleContextMenu(e, device.id)}
                    className={`w-full flex items-center gap-2 px-2 py-1 rounded-md transition-all text-left group ${
                      isActive
                        ? isDark ? "bg-blue-900/30 text-blue-400" : "bg-blue-50 text-blue-600"
                        : isDark ? "text-gray-200 hover:bg-gray-800" : "text-gray-700 hover:bg-gray-50"
                    }`}
                  >
                    <div className="relative shrink-0">
                      <DeviceIcon style={{ width: 14, height: 14 }} className={isActive ? (isDark ? "text-blue-400" : "text-blue-600") : (isDark ? "text-gray-300" : "text-gray-500")} />
                      <div className={`absolute -bottom-0.5 -right-0.5 w-2 h-2 rounded-full border-[1.5px] bg-green-500 ${isDark ? "border-[#1e1e1e]" : "border-[#e9ecf2]"}`} />
                    </div>
                    {isEditing ? (
                      <div className="flex items-center gap-1 flex-1 min-w-0">
                      <input
                        ref={editInputRef}
                        type="text"
                        value={editingName}
                        onChange={(e) => setEditingName(e.target.value)}
                        onKeyDown={handleKeyDown}
                        className={`flex-1 min-w-0 px-1.5 py-0.5 rounded text-xs outline-none border ${
                          isDark ? "bg-[#1a1a1a] border-blue-500 text-gray-200" : "bg-white border-blue-400 text-gray-900"
                        }`}
                      />
                      <button
                        onClick={handleConfirmRename}
                        className={`shrink-0 p-0.5 rounded transition-colors ${isDark ? "text-gray-500 hover:text-green-400" : "text-gray-400 hover:text-green-600"}`}
                      >
                        <Check style={{ width: 12, height: 12 }} />
                      </button>
                      <button
                        onClick={handleCancelRename}
                        className={`shrink-0 p-0.5 rounded transition-colors ${isDark ? "text-gray-500 hover:text-red-400" : "text-gray-400 hover:text-red-600"}`}
                      >
                        <X style={{ width: 12, height: 12 }} />
                      </button>
                      </div>
                    ) : (
                      <>
                        <button
                          onClick={() => !renameSaved && navigate(`/devices/${device.id}`)}
                          draggable
                          onDragStart={(e) => e.dataTransfer.setData("deviceId", device.id)}
                          className="flex-1 min-w-0 truncate text-left"
                          style={{ fontSize: 12 }}
                        >
                          {device.name}
                        </button>
                        {isCurrentDevice && (
                          <span className={`shrink-0 px-1 rounded text-[9px] ${isDark ? "bg-blue-900/50 text-blue-400" : "bg-blue-100 text-blue-600"}`}>
                            本机
                          </span>
                        )}
                        {device.ping !== null && (
                          <span className={`shrink-0 ${device.ping < 30 ? "text-green-600" : "text-yellow-600"}`} style={{ fontSize: 10 }}>
                            {device.ping}ms
                          </span>
                        )}
                        <button type="button" aria-label={`${device.name}的更多操作`}
                          className={`shrink-0 p-0.5 rounded opacity-60 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity ${isDark ? "hover:bg-gray-700" : "hover:bg-gray-200"}`}
                          onClick={(e) => handleMoreClick(e, device.id)}
                        >
                          <MoreHorizontal size={14} aria-hidden="true" />
                        </button>
                      </>
                    )}
                    {renameSaved && !isEditing && (
                      <span className="shrink-0 text-green-600" style={{ fontSize: 10 }}>
                        <Check style={{ width: 10, height: 10 }} />
                      </span>
                    )}
                  </div>
                );
              })}

              {/* Divider */}
              {offlineDevices.length > 0 && onlineDevices.length > 0 && (
                <div className="flex items-center gap-2 px-2 py-1">
                  <div className={`flex-1 h-px ${isDark ? "bg-gray-700" : "bg-gray-100"}`} />
                  <span className="text-gray-400" style={{ fontSize: 9 }}>离线</span>
                  <div className={`flex-1 h-px ${isDark ? "bg-gray-700" : "bg-gray-100"}`} />
                </div>
              )}

              {/* Offline devices */}
              {offlineDevices.map((device) => {
                const isActive = location.pathname === `/devices/${device.id}`;
                const DeviceIcon = device.icon;
                const isCurrentDevice = device.deviceId === currentDeviceId;
                const isEditing = editingDeviceId === device.id;
                return (
                  <div
                    key={device.id}
                    onContextMenu={(e) => !isEditing && handleContextMenu(e, device.id)}
                    className={`w-full flex items-center gap-2 px-2 py-1 rounded-md transition-all text-left group ${
                      isActive
                        ? isDark ? "bg-gray-800 text-gray-300" : "bg-gray-100 text-gray-600"
                        : isDark ? "text-gray-400 hover:bg-gray-800 hover:text-gray-300" : "text-gray-500 hover:bg-gray-50 hover:text-gray-600"
                    }`}
                  >
                    <div className="relative shrink-0">
                      <DeviceIcon style={{ width: 14, height: 14 }} className={isDark ? "text-gray-500" : "text-gray-400"} />
                      <div className={`absolute -bottom-0.5 -right-0.5 w-2 h-2 rounded-full border-[1.5px] bg-gray-300 ${isDark ? "border-[#1e1e1e]" : "border-[#e9ecf2]"}`} />
                    </div>
                    {isEditing ? (
                      <div className="flex items-center gap-1 flex-1 min-w-0">
                        <input
                          ref={editInputRef}
                          type="text"
                          value={editingName}
                          onChange={(e) => setEditingName(e.target.value)}
                          onKeyDown={handleKeyDown}
                          className={`flex-1 min-w-0 px-1.5 py-0.5 rounded text-xs outline-none border ${
                            isDark ? "bg-[#1a1a1a] border-blue-500 text-gray-200" : "bg-white border-blue-400 text-gray-900"
                          }`}
                        />
                        <button
                          onClick={handleConfirmRename}
                          className={`shrink-0 p-0.5 rounded transition-colors ${isDark ? "text-gray-500 hover:text-green-400" : "text-gray-400 hover:text-green-600"}`}
                        >
                          <Check style={{ width: 12, height: 12 }} />
                        </button>
                        <button
                          onClick={handleCancelRename}
                          className={`shrink-0 p-0.5 rounded transition-colors ${isDark ? "text-gray-500 hover:text-red-400" : "text-gray-400 hover:text-red-600"}`}
                        >
                          <X style={{ width: 12, height: 12 }} />
                        </button>
                      </div>
                    ) : (
                      <>
                        <button
                          onClick={() => !renameSaved && navigate(`/devices/${device.id}`)}
                          className="flex-1 min-w-0 truncate text-left"
                          style={{ fontSize: 12 }}
                        >
                          {device.name}
                        </button>
                        {isCurrentDevice && (
                          <span className={`shrink-0 px-1 rounded text-[9px] ${isDark ? "bg-blue-900/50 text-blue-400" : "bg-blue-100 text-blue-600"}`}>
                            本机
                          </span>
                        )}
                        <span className="shrink-0 text-gray-400" style={{ fontSize: 10 }}>{device.lastSeen}</span>
                        <button type="button" aria-label={`${device.name}的更多操作`}
                          className={`shrink-0 p-0.5 rounded opacity-60 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity ${isDark ? "hover:bg-gray-700" : "hover:bg-gray-200"}`}
                          onClick={(e) => handleMoreClick(e, device.id)}
                        >
                          <MoreHorizontal size={14} aria-hidden="true" />
                        </button>
                      </>
                    )}
                    {renameSaved && !isEditing && (
                      <span className="shrink-0 text-green-600" style={{ fontSize: 10 }}>
                        <Check style={{ width: 10, height: 10 }} />
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}

      {/* Collapsed: just show device dots */}
      {collapsed && (
        <div className={`flex-1 overflow-y-auto border-t py-2 px-1 space-y-1 ${isDark ? "border-gray-700" : "border-gray-100"}`}>
          {devices.map((device) => {
            const isActive = location.pathname === `/devices/${device.id}`;
            const DeviceIcon = device.icon;
            const isCurrentDevice = device.deviceId === currentDeviceId;
            return (
              <button
                key={device.id}
                onClick={() => navigate(`/devices/${device.id}`)}
                onContextMenu={(e) => handleContextMenu(e, device.id)}
                className={`w-full flex items-center justify-center p-2 rounded-md transition-all relative ${
                  isActive ? (isDark ? "bg-blue-900/30" : "bg-blue-50") : (isDark ? "hover:bg-gray-800" : "hover:bg-gray-50")
                }`}
                title={`${device.name} (${device.status === "online" ? "在线" : "离线"})${isCurrentDevice ? " - 本机" : ""}`}
              >
                <div className="relative">
                  <DeviceIcon
                    style={{ width: 15, height: 15 }}
                    className={isActive ? (isDark ? "text-blue-400" : "text-blue-600") : device.status === "online" ? (isDark ? "text-gray-300" : "text-gray-600") : "text-gray-400"}
                  />
                  <div className={`absolute -bottom-0.5 -right-1 w-2 h-2 rounded-full border-[1.5px] ${isDark ? "border-[#1e1e1e]" : "border-[#e9ecf2]"} ${
                    device.status === "online" ? "bg-green-500" : "bg-gray-300"
                  }`} />
                  {isCurrentDevice && (
                    <div className={`absolute -top-1 -right-1 w-2.5 h-2.5 rounded-full ${isDark ? "bg-blue-500" : "bg-blue-600"}`} />
                  )}
                </div>
              </button>
            );
          })}
        </div>
      )}

      {actionStatus && !collapsed && (
        <div
          role="status"
          className={`mx-2 mb-2 rounded-md border px-2 py-1 text-xs ${
            actionStatus.kind === "success"
              ? isDark ? "border-green-900/60 bg-green-950/40 text-green-300" : "border-green-200 bg-green-50 text-green-700"
              : isDark ? "border-red-900/60 bg-red-950/40 text-red-300" : "border-red-200 bg-red-50 text-red-700"
          }`}
        >
          {actionStatus.message}
        </div>
      )}

      {native && <div className="px-2 py-1">
        <NavLink to="/test" className="remote-nav-link" aria-label="测试工作台" title="测试工作台">
          <Terminal size={16} aria-hidden="true" />
          {!collapsed && <span>测试工作台</span>}
        </NavLink>
      </div>}
      <AppVersionBadge collapsed={collapsed} isDark={isDark} />

      {/* Context menu */}
      {contextMenu && contextMenuDevice && (
        <div
          ref={contextMenuRef}
          className={`fixed z-50 min-w-[160px] rounded-lg border py-1 shadow-xl ${
            isDark ? "bg-[#2a2a2a] border-gray-700" : "bg-white border-gray-200"
          }`}
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onMouseLeave={() => setSubmenuOpen(null)}
        >
          {getTopLevelMenuItems().map((item, index) => {
            if (item.type === "divider") {
              return (
                <div key={index} className={`h-px my-1 mx-2 ${isDark ? "bg-gray-700" : "bg-gray-100"}`} />
              );
            }

            const ItemIcon = item.icon;
            const isDanger = item.danger;
            const hasSubmenu = item.submenu === "management";
            const isDisabled = Boolean(item.disabled);

            return (
              <div key={index} className="relative">
                <button
                  className={`w-full flex items-center justify-between gap-2.5 px-3 py-1.5 text-left transition-colors ${
                    isDisabled
                      ? isDark ? "text-gray-600 cursor-not-allowed" : "text-gray-400 cursor-not-allowed"
                      : isDanger
                      ? isDark ? "text-red-400 hover:bg-red-900/30" : "text-red-500 hover:bg-red-50"
                      : isDark ? "text-gray-300 hover:bg-gray-700" : "text-gray-600 hover:bg-gray-50"
                  }`}
                  style={{ fontSize: 12 }}
                  disabled={isDisabled}
                  title={item.title}
                  onClick={() => { if (item.action) void item.action(); }}
                  onMouseEnter={() => hasSubmenu && setSubmenuOpen("management")}
                >
                  <span className="flex items-center gap-2.5">
                    {ItemIcon && <ItemIcon style={{ width: 14, height: 14 }} className="shrink-0" />}
                    <span>{item.label}</span>
                  </span>
                  {hasSubmenu && <ChevronDown className="w-3 h-3 -rotate-90" />}
                </button>

                {/* 管理二级菜单 */}
                {hasSubmenu && submenuOpen === "management" && (
                  <div
                    className={`absolute left-full top-0 ml-1 min-w-[140px] rounded-lg border py-1 shadow-xl ${
                      isDark ? "bg-[#2a2a2a] border-gray-700" : "bg-white border-gray-200"
                    }`}
                    onMouseEnter={() => setSubmenuOpen("management")}
                  >
                    {getManagementSubmenuItems().map((subItem, subIndex) => {
                      const SubIcon = subItem.icon;
                      return (
                        <button
                          key={subIndex}
                          disabled={subItem.disabled}
                          title={subItem.title}
                          onClick={() => { if (subItem.action) void subItem.action(); }}
                          className={`w-full flex items-center gap-2.5 px-3 py-1.5 text-left transition-colors ${
                            subItem.disabled
                              ? isDark ? "text-gray-600 cursor-not-allowed" : "text-gray-400 cursor-not-allowed"
                              : isDark ? "text-gray-300 hover:bg-gray-700" : "text-gray-600 hover:bg-gray-50"
                          }`}
                          style={{ fontSize: 12 }}
                        >
                          {SubIcon && <SubIcon style={{ width: 14, height: 14 }} className="shrink-0" />}
                          <span>{subItem.label}</span>
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </aside>
  );
}
