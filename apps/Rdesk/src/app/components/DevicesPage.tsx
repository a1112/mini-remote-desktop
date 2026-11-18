import { useState, useEffect, useRef } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import {
  Plus,
  Search,
  MoreVertical,
  Monitor,
  Star,
  Trash2,
  Edit2,
  ExternalLink,
  Wifi,
  WifiOff,
  MapPin,
  Clock,
  Power,
  RefreshCw,
  RotateCw,
  Info,
  Zap,
  X,
} from "lucide-react";
import { useDevices, Device } from "./deviceData";
import { useTheme } from "./ThemeContext";
import { useAuth } from "./AuthContext";
import { isTauriRuntime } from "../utils/runtime";
import { NetworkGroupEditModal } from "./NetworkGroupEditModal";
import { LanPairingPanel } from "./LanPairingPanel";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from "./ui/context-menu";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./ui/dropdown-menu";
import { useNetworkGroups } from "../hooks/useNetworkGroups";
import { networkGroupService } from "../services/networkGroupService";
import {
  launchLocalRemoteDisplayTest,
  launchRemoteDisplayForDevice,
} from "../services/remoteDisplayLauncher";
import { normalizeDeviceCode } from "../utils/deviceCode";

const unavailableManagementActions = [
  { icon: Edit2, label: "重命名" },
  { icon: Star, label: "收藏设备" },
  { icon: Power, label: "启用或禁用设备" },
  { icon: Trash2, label: "移除设备" },
  { icon: RotateCw, label: "重启" },
  { icon: Power, label: "关机" },
  { icon: Zap, label: "网络唤醒" },
];

type GroupMembership = {
  groupId: string;
  devices: Map<string, boolean>;
  error: string | null;
};

function DeviceMenuItems({
  Item,
  connectionLabel,
  unavailable,
  onConnect,
  detailPath,
}: {
  Item: typeof DropdownMenuItem | typeof ContextMenuItem;
  connectionLabel: string;
  unavailable: string | null;
  onConnect: () => void;
  detailPath: string;
}) {
  return <>
    <Item disabled={Boolean(unavailable)} onSelect={onConnect}>
      <Monitor aria-hidden="true" />
      {connectionLabel}
      {unavailable && <span className="ml-auto pl-4 text-xs">{unavailable}</span>}
    </Item>
    <Item asChild>
      <Link to={`${detailPath}?tab=info`}>
        <Info aria-hidden="true" />
        设备信息
      </Link>
    </Item>
    <div role="separator" className="my-1 border-t border-border" />
    {unavailableManagementActions.map(({ icon: Icon, label }) => (
      <Item key={label} disabled>
        <Icon aria-hidden="true" />
        {label}
        <span className="ml-auto pl-4 text-xs">暂不可用</span>
      </Item>
    ))}
  </>;
}

export function DevicesPage() {
  const native = isTauriRuntime();
  const { isLoggedIn, token, user } = useAuth();
  const { isDark } = useTheme();
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const search = searchParams.get("q") ?? "";
  const { devices, loading, error, refresh, lastUpdated, currentDeviceId } = useDevices({
    pollInterval: 30000,
    enabled: true,
  });
  const {
    groups,
    error: groupListError,
    lastUpdated: groupListUpdatedAt,
    refresh: refreshGroups,
  } = useNetworkGroups({ pollInterval: 30000, enabled: true });

  const [refreshing, setRefreshing] = useState(false);
  const [selectedGroupId, setSelectedGroupId] = useState<string | null>(null);
  const [editingGroupId, setEditingGroupId] = useState<string | null>(null);
  const [groupMembership, setGroupMembership] = useState<GroupMembership | null>(null);
  const [membershipAttempt, setMembershipAttempt] = useState(0);
  const [createGroupModalOpen, setCreateGroupModalOpen] = useState(false);
  const [newGroupName, setNewGroupName] = useState("");
  const [newGroupDescription, setNewGroupDescription] = useState("");
  const [launchingDeviceId, setLaunchingDeviceId] = useState<string | null>(null);
  const [launchError, setLaunchError] = useState<string | null>(null);
  const launchInFlight = useRef(false);
  const [lanPairingOpen, setLanPairingOpen] = useState(false);

  const accountGroups = isLoggedIn && token ? groups.filter((group) => group.user_id === user?.id) : [];
  const currentGroup = accountGroups.find((group) => group.id === selectedGroupId);
  const activeGroupId = currentGroup?.id ?? null;
  const activeMembership = groupMembership?.groupId === activeGroupId ? groupMembership : null;

  useEffect(() => {
    if (!activeGroupId || editingGroupId) return;
    let cancelled = false;
    void networkGroupService.getGroupDevices(activeGroupId).then((members) => {
      if (cancelled) return;
      setGroupMembership({
        groupId: activeGroupId,
        devices: new Map(members.map((member) => [normalizeDeviceCode(member.device_id), member.is_enabled])),
        error: null,
      });
    }).catch((cause: unknown) => {
      if (!cancelled) {
        setGroupMembership({ groupId: activeGroupId, devices: new Map(), error: cause instanceof Error ? cause.message : "加载分组设备失败" });
      }
    });
    return () => { cancelled = true; };
  }, [activeGroupId, editingGroupId, groupListUpdatedAt, membershipAttempt, token]);

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      await Promise.all([refresh(), refreshGroups()]);
    } finally {
      setTimeout(() => setRefreshing(false), 500);
    }
  };

  const query = search.trim().toLowerCase();
  const codeQuery = normalizeDeviceCode(query);
  const filteredDevices = devices.filter((device) => {
    const deviceCode = normalizeDeviceCode(device.deviceId);
    if (activeGroupId && !activeMembership?.devices.has(deviceCode)) return false;
    return !query || deviceCode.toLowerCase().includes(codeQuery) ||
      [device.name, device.os, device.ip].some((value) => value.toLowerCase().includes(query));
  });

  const connectionUnavailable = (device: Device) => {
    if (device.disabled) return "设备已禁用";
    if (currentGroup?.is_enabled === false) return "分组已停用";
    if (activeMembership?.devices.get(normalizeDeviceCode(device.deviceId)) === false) return "设备在此分组中已禁用";
    if (device.status === "offline") return "设备离线";
    return null;
  };

  const updateSearch = (value: string) => {
    const next = new URLSearchParams(searchParams);
    if (value) next.set("q", value);
    else next.delete("q");
    setSearchParams(next, { replace: true });
  };

  const card = isDark ? "bg-[#232323] border-gray-700" : "bg-white border-gray-200/70 shadow-sm";
  const cardHover = isDark ? "hover:border-gray-600 hover:shadow-sm" : "hover:border-gray-300 hover:shadow-md";
  const textPrimary = isDark ? "text-gray-100" : "text-gray-900";
  const textTertiary = isDark ? "text-gray-500" : "text-gray-400";
  const textBody = isDark ? "text-gray-200" : "text-gray-800";
  const inputBg = isDark
    ? "bg-[#2a2a2a] border-gray-600 text-gray-200 placeholder-gray-500"
    : "bg-[#f7f8fa] border-gray-200 text-gray-900 placeholder-gray-400";
  const sourceBadgeClass = (source: Device["discoverySources"][number]) => {
    if (source === "local") {
      return isDark ? "bg-blue-900/50 text-blue-300" : "bg-blue-100 text-blue-700";
    }
    if (source === "lan_p2p") {
      return isDark ? "bg-emerald-900/50 text-emerald-300" : "bg-emerald-100 text-emerald-700";
    }
    return isDark ? "bg-gray-800 text-gray-300" : "bg-gray-100 text-gray-700";
  };
  const sourceBadgeText: Record<Device["discoverySources"][number], string> = {
    local: "本机",
    lan_p2p: "P2P 局域网",
    server: "服务器",
  };

  // 创建新分组
  const handleCreateGroup = async () => {
    if (!newGroupName.trim()) {
      alert("请输入分组名称");
      return;
    }

    try {
      await networkGroupService.createNetworkGroup({
        name: newGroupName.trim(),
        description: newGroupDescription.trim() || undefined,
      });
      await refreshGroups();
      setCreateGroupModalOpen(false);
      setNewGroupName("");
      setNewGroupDescription("");
    } catch (e) {
      alert("创建分组失败: " + (e instanceof Error ? e.message : "未知错误"));
    }
  };

  // 打开创建分组弹窗
  const handleOpenCreateModal = () => {
    setNewGroupName("");
    setNewGroupDescription("");
    setCreateGroupModalOpen(true);
  };

  const closeGroupEditor = () => {
    setEditingGroupId(null);
    setGroupMembership(null);
  };

  const launchDisplay = async (device: Device | null) => {
    if (launchInFlight.current || (device && connectionUnavailable(device))) return;
    const localTest = !device || device.isLocal || device.deviceId === currentDeviceId;
    launchInFlight.current = true;
    setLaunchingDeviceId(device?.id ?? "__local_test__");
    setLaunchError(null);
    try {
      const result = device && !localTest
        ? await launchRemoteDisplayForDevice(device.deviceId, {
          transportKind: device.p2pAvailable || device.os.toLowerCase().includes("quic") ? "quic" : "webrtc",
          targetDeviceName: device.name,
          targetOs: device.os,
          targetIp: device.ip,
          lanP2P: device.p2pAvailable,
        })
        : await launchLocalRemoteDisplayTest();
      if (result.mode === "route") {
        const page = localTest ? "display" : "session";
        navigate(result.routePath ?? `/${page}/${result.sessionId}`);
      }
    } catch (error) {
      const action = localTest ? "打开本机测试窗口失败" : `连接 ${device.name} 失败`;
      const message = error instanceof Error ? error.message : "请稍后重试";
      setLaunchError(`${action}。${message}`);
    } finally {
      launchInFlight.current = false;
      setLaunchingDeviceId(null);
    }
  };

  return (
    <div className="flex min-h-full">
      <div className="remote-devices-content min-w-0 flex-1">
        <header className="remote-page-header"><h1>我的设备</h1></header>
        {/* Toolbar */}
        <div className="mb-5 flex flex-wrap items-center gap-3">
          {/* 搜索框 */}
          <div className="relative min-w-44 max-w-xs flex-1">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400" />
            <input
              type="search"
              aria-label="搜索设备"
              value={search}
              onChange={(e) => updateSearch(e.target.value)}
              placeholder="搜索设备、设备码或 IP"
              className={`w-full pl-9 pr-3 py-1.5 rounded-lg border outline-none transition-all focus-visible:ring-2 focus-visible:ring-blue-500 ${inputBg}`}
              style={{ fontSize: 13 }}
            />
          </div>

          {accountGroups.length > 0 && (
            <select
              aria-label="按网络分组筛选"
              value={activeGroupId ?? ""}
              onChange={(event) => {
                setSelectedGroupId(event.target.value || null);
                setGroupMembership(null);
              }}
              className={`max-w-48 rounded-lg border px-3 py-1.5 text-[13px] focus-visible:ring-2 focus-visible:ring-blue-500 ${inputBg}`}
            >
              <option value="">全部设备</option>
              {accountGroups.map((group) => <option key={group.id} value={group.id}>{group.name}</option>)}
            </select>
          )}

          {activeGroupId && (
            <button
              onClick={() => setEditingGroupId(activeGroupId)}
              className={`flex items-center gap-2 px-3 py-1.5 rounded-lg border transition-colors ${
                isDark
                  ? "bg-[#232323] border-gray-600 text-gray-300 hover:border-gray-500"
                  : "bg-white border-gray-200 text-gray-700 hover:border-gray-300"
              }`}
              style={{ fontSize: 13 }}
            >
              <Edit2 className="w-3.5 h-3.5" />
              编辑分组
            </button>
          )}

          {isLoggedIn && (
            <button
              onClick={handleOpenCreateModal}
              className={`flex items-center gap-2 px-3.5 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-white transition-colors shadow-sm`}
              style={{ fontSize: 13 }}
            >
              <Plus className="w-3.5 h-3.5" />
              添加分组
            </button>
          )}

          <div className="flex-1" />
          {lastUpdated && (
            <span
              className={`text-xs mr-2 ${isDark ? "text-gray-500" : "text-gray-400"}`}
            >
              更新于 {lastUpdated.toLocaleTimeString()}
            </span>
          )}
          <button
            onClick={handleRefresh}
            disabled={refreshing}
            className={`p-2 rounded-lg transition-colors ${
              isDark
                ? "text-gray-400 hover:text-gray-200 hover:bg-gray-700"
                : "text-gray-400 hover:text-gray-600 hover:bg-gray-100"
            } ${refreshing ? "animate-spin" : ""}`}
            title="刷新"
          >
            <RefreshCw className="w-5 h-5" />
          </button>
          {native && <>
            <button
              onClick={() => void launchDisplay(null)}
              disabled={launchingDeviceId !== null}
              className={`flex items-center gap-2 px-3.5 py-1.5 rounded-lg border transition-colors ${
                isDark
                  ? "bg-[#232323] border-gray-600 text-gray-200 hover:border-gray-500"
                  : "bg-white border-gray-200 text-gray-700 hover:border-gray-300"
              } disabled:cursor-not-allowed disabled:opacity-60`}
              style={{ fontSize: 13 }}
            >
              <Monitor className="w-3.5 h-3.5" />
              {launchingDeviceId === "__local_test__" ? "正在打开…" : "测试窗口"}
            </button>
            <button
              aria-expanded={lanPairingOpen}
              onClick={() => setLanPairingOpen(open => !open)}
              className="flex items-center gap-2 px-3.5 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-white transition-colors shadow-sm"
              style={{ fontSize: 13 }}
            >
              <Wifi className="w-3.5 h-3.5" />
              局域网配对
            </button>
          </>}
        </div>

        {native && lanPairingOpen ? <LanPairingPanel isDark={isDark} onPaired={refresh} /> : null}

        {loading ? <p role="status" className={`mb-3 ${textPrimary}`}>加载设备中...</p> : null}
        {error ? <p role="alert" className="mb-3 text-red-500">设备加载失败: {error}</p> : null}
        {isLoggedIn && groupListError ? <p role="alert" className="mb-3 text-red-500">{groupListError}</p> : null}
        {activeGroupId && !activeMembership ? <p role="status" className={`mb-3 ${textPrimary}`}>正在读取分组设备…</p> : null}
        {activeMembership?.error ? (
          <div role="alert" className="mb-3 flex items-center gap-3 text-red-500">
            <span>{activeMembership.error}</span>
            <button type="button" className="underline" onClick={() => {
              setGroupMembership(null);
              setMembershipAttempt((attempt) => attempt + 1);
            }}>重试</button>
          </div>
        ) : null}
        {launchError ? <p role="alert" className={`mb-3 ${isDark ? "text-red-300" : "text-red-700"}`}>{launchError}</p> : null}
        {!loading && !error && (!activeGroupId || (activeMembership && !activeMembership.error)) && filteredDevices.length === 0 ? (
          <p role="status" className={`rounded-xl border p-6 text-sm ${card} ${textPrimary}`}>
            {query ? "没有符合条件的设备" : activeGroupId ? "此分组暂无可显示的设备" : "暂无设备"}
          </p>
        ) : null}

        {/* 设备列表 */}
        <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
          {filteredDevices.map((device) => {
            const Icon = device.icon;
            const isCurrentDevice = device.deviceId === currentDeviceId;
            const localTest = device.isLocal || isCurrentDevice;
            const connectionLabel = localTest ? "测试本机" : device.p2pAvailable ? "P2P 连接" : "远程连接";
            const unavailable = connectionUnavailable(device);
            const menu = {
              connectionLabel: localTest ? "测试本机" : "远程桌面",
              unavailable: unavailable ?? (launchingDeviceId !== null ? "连接处理中" : null),
              detailPath: `/devices/${encodeURIComponent(device.id)}`,
              onConnect: () => void launchDisplay(device),
            };
            const deviceCard = (
              <article
                aria-label={device.name}
                className={`relative p-4 rounded-xl border transition-all ${card} ${cardHover}`}
              >
                <div className="flex items-start justify-between mb-3">
                  <div className="flex items-center gap-3">
                    <div
                      className={`relative w-10 h-10 rounded-xl flex items-center justify-center ${
                        device.status === "online"
                          ? isDark
                            ? "bg-blue-900/30"
                            : "bg-blue-50"
                          : isDark
                            ? "bg-gray-800"
                            : "bg-gray-100"
                      }`}
                    >
                      <Icon
                        style={{ width: 20, height: 20 }}
                        className={device.status === "online" ? "text-blue-600" : "text-gray-400"}
                      />
                      <div
                        className={`absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full border-2 ${
                          isDark ? "border-[#232323]" : "border-white"
                        } ${device.status === "online" ? "bg-green-500" : "bg-gray-300"}`}
                      />
                    </div>
                    <div>
                      <div className="flex items-center gap-1.5">
                        <span className={`font-medium ${textBody}`} style={{ fontSize: 14 }}>
                          {device.name}
                        </span>
                        {isCurrentDevice && !device.isLocal && (
                          <span
                            className={`px-1.5 py-0.5 rounded text-[10px] ${
                              isDark
                                ? "bg-blue-900/50 text-blue-400"
                                : "bg-blue-100 text-blue-600"
                            }`}
                          >
                            本机
                          </span>
                        )}
                        {device.discoverySources.map((source) => (
                          <span
                            key={source}
                            className={`px-1.5 py-0.5 rounded text-[10px] ${sourceBadgeClass(source)}`}
                          >
                            {sourceBadgeText[source]}
                          </span>
                        ))}
                        {device.favorite && <Star className="w-3 h-3 text-yellow-500 fill-yellow-500" />}
                      </div>
                      <span className={textTertiary} style={{ fontSize: 12 }}>
                        {device.os} · {device.sourceLabel}
                      </span>
                    </div>
                  </div>

                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <button
                        type="button"
                        aria-label={`${device.name}的更多操作`}
                        className={`relative z-10 p-1 rounded-md ${
                          isDark
                            ? "text-gray-400 hover:text-gray-200 hover:bg-gray-700"
                            : "text-gray-600 hover:text-gray-800 hover:bg-gray-100"
                        }`}
                      >
                        <MoreVertical aria-hidden="true" className="w-4 h-4" />
                      </button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="min-w-56">
                      <DeviceMenuItems Item={DropdownMenuItem} {...menu} />
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>

                <div className="flex items-center gap-3 mb-3">
                  <div className={`flex items-center gap-1 ${textTertiary}`} style={{ fontSize: 11 }}>
                    <MapPin className="w-3 h-3" />
                    {device.location}
                  </div>
                  <span className={isDark ? "text-gray-600" : "text-gray-300"} style={{ fontSize: 11 }}>
                    ·
                  </span>
                  <div className={`flex items-center gap-1 ${textTertiary}`} style={{ fontSize: 11 }}>
                    <Clock className="w-3 h-3" />
                    {device.lastSeen}
                  </div>
                  {device.ping !== null && (
                    <>
                      <span
                        className={isDark ? "text-gray-600" : "text-gray-300"}
                        style={{ fontSize: 11 }}
                      >
                        ·
                      </span>
                      <div
                        className={`flex items-center gap-1 ${
                          device.ping < 30 ? "text-green-600" : "text-yellow-600"
                        }`}
                        style={{ fontSize: 11 }}
                      >
                        <Wifi className="w-3 h-3" />
                        {device.ping}ms
                      </div>
                    </>
                  )}
                </div>

                {/* Resource bars */}
                {device.status === "online" && device.cpu !== null && (
                  <div className="space-y-1.5 mb-3">
                    {[
                      { label: "CPU", value: device.cpu, color: "bg-blue-500" },
                      { label: "内存", value: device.ram!, color: "bg-purple-500" },
                      { label: "磁盘", value: device.disk!, color: "bg-green-500" },
                    ].map(({ label, value, color }) => (
                      <div key={label} className="flex items-center gap-2">
                        <span className={`w-6 shrink-0 ${textTertiary}`} style={{ fontSize: 10 }}>
                          {label}
                        </span>
                        <div className={`flex-1 h-1 rounded-full ${isDark ? "bg-gray-700" : "bg-gray-200"}`}>
                          <div
                            className={`h-full rounded-full ${color}`}
                            style={{ width: `${value}%`, opacity: 0.75 }}
                          />
                        </div>
                        <span className={`${textTertiary} w-7 text-right shrink-0`} style={{ fontSize: 10 }}>
                          {value}%
                        </span>
                      </div>
                    ))}
                  </div>
                )}

                <div className="space-y-2">
                  {unavailable ? (
                    <div className={`flex items-center justify-center gap-1.5 rounded-lg py-2 text-center text-[13px] ${
                      isDark ? "bg-gray-800 text-gray-400" : "bg-gray-50 text-gray-600"
                    }`}>
                      <WifiOff aria-hidden="true" className="w-3.5 h-3.5" />
                      {unavailable}
                    </div>
                  ) : (
                    <button
                      type="button"
                      onClick={menu.onConnect}
                      disabled={launchingDeviceId !== null}
                      aria-label={`${connectionLabel} ${device.name}`}
                      aria-busy={launchingDeviceId === device.id}
                      className={`relative z-10 w-full py-2 rounded-lg transition-colors flex items-center justify-center gap-1.5 ${
                        isDark
                          ? "bg-emerald-900/30 hover:bg-emerald-900/50 text-emerald-300"
                          : "bg-emerald-50 hover:bg-emerald-100 text-emerald-700"
                      } disabled:cursor-not-allowed disabled:opacity-60`}
                      style={{ fontSize: 13 }}
                    >
                      <Monitor aria-hidden="true" className="w-3.5 h-3.5" />
                      {launchingDeviceId === device.id ? "正在打开…" : connectionLabel}
                    </button>
                  )}
                  <Link
                    to={menu.detailPath}
                    aria-label={`查看详情 ${device.name}`}
                    className={`w-full py-2 rounded-lg transition-colors flex items-center justify-center gap-1.5 after:absolute after:inset-0 after:rounded-xl focus-visible:outline-none focus-visible:after:ring-2 focus-visible:after:ring-blue-500 ${
                      isDark
                        ? "bg-blue-900/30 hover:bg-blue-900/50 text-blue-400"
                        : "bg-blue-50 hover:bg-blue-100 text-blue-600"
                    }`}
                    style={{ fontSize: 13 }}
                  >
                    <ExternalLink aria-hidden="true" className="w-3.5 h-3.5" />
                    查看详情
                  </Link>
                </div>
              </article>
            );
            return (
              <ContextMenu key={device.id}>
                <ContextMenuTrigger asChild>{deviceCard}</ContextMenuTrigger>
                <ContextMenuContent className="min-w-56">
                  <DeviceMenuItems Item={ContextMenuItem} {...menu} />
                </ContextMenuContent>
              </ContextMenu>
            );
          })}
        </div>
      </div>

      {/* 编辑弹窗 */}
      {editingGroupId && (
        <NetworkGroupEditModal
          group={accountGroups.find((g) => g.id === editingGroupId) ?? null}
          allDevices={devices.filter((d) => d.serverAvailable).map((d) => ({
            id: d.id,
            name: d.name,
            device_id: d.deviceId,
            status: d.status,
            ip: d.ip,
          }))}
          isOpen
          onClose={closeGroupEditor}
          onSave={() => {
            closeGroupEditor();
            void refreshGroups();
          }}
          isDark={isDark}
        />
      )}

      {/* 创建分组弹窗 */}
      {createGroupModalOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div
            className={`w-[400px] rounded-xl border shadow-2xl ${
              isDark ? "bg-[#232323] border-gray-700" : "bg-white border-gray-200"
            }`}
          >
            {/* 标题栏 */}
            <div className={`flex items-center justify-between px-5 py-4 border-b ${isDark ? "border-gray-700" : "border-gray-200"}`}>
              <h2 className={`text-lg font-semibold ${isDark ? "text-gray-100" : "text-gray-900"}`}>
                创建网络分组
              </h2>
              <button
                onClick={() => setCreateGroupModalOpen(false)}
                className={`p-1 rounded-md transition-colors ${
                  isDark ? "hover:bg-gray-700 text-gray-400" : "hover:bg-gray-100 text-gray-500"
                }`}
              >
                <X className="w-5 h-5" />
              </button>
            </div>

            {/* 内容 */}
            <div className="p-5 space-y-4">
              <div>
                <label className={`block mb-2 ${isDark ? "text-gray-400" : "text-gray-500"}`} style={{ fontSize: 12 }}>
                  分组名称 <span className="text-red-500">*</span>
                </label>
                <input
                  value={newGroupName}
                  onChange={(e) => setNewGroupName(e.target.value)}
                  placeholder="请输入分组名称"
                  className={`w-full px-3 py-2 rounded-lg border outline-none ${
                    isDark
                      ? "bg-[#2a2a2a] border-gray-600 text-gray-200 placeholder-gray-500"
                      : "bg-gray-50 border-gray-200 text-gray-900 placeholder-gray-400"
                  }`}
                  style={{ fontSize: 13 }}
                  autoFocus
                />
              </div>
              <div>
                <label className={`block mb-2 ${isDark ? "text-gray-400" : "text-gray-500"}`} style={{ fontSize: 12 }}>
                  分组描述
                </label>
                <textarea
                  value={newGroupDescription}
                  onChange={(e) => setNewGroupDescription(e.target.value)}
                  placeholder="请输入分组描述（可选）"
                  className={`w-full px-3 py-2 rounded-lg border outline-none ${
                    isDark
                      ? "bg-[#2a2a2a] border-gray-600 text-gray-200 placeholder-gray-500"
                      : "bg-gray-50 border-gray-200 text-gray-900 placeholder-gray-400"
                  }`}
                  rows={3}
                  style={{ fontSize: 13 }}
                />
              </div>
            </div>

            {/* 底部按钮 */}
            <div className={`flex items-center justify-end gap-3 px-5 py-4 border-t ${isDark ? "border-gray-700" : "border-gray-200"}`}>
              <button
                onClick={() => setCreateGroupModalOpen(false)}
                className={`px-4 py-2 rounded-lg border transition-colors ${
                  isDark
                    ? "border-gray-600 text-gray-300 hover:bg-gray-700"
                    : "border-gray-200 text-gray-600 hover:bg-gray-50"
                }`}
                style={{ fontSize: 13 }}
              >
                取消
              </button>
              <button
                onClick={handleCreateGroup}
                disabled={!newGroupName.trim()}
                className={`px-4 py-2 rounded-lg bg-blue-600 hover:bg-blue-500 text-white transition-colors ${
                  !newGroupName.trim() ? "opacity-50 cursor-not-allowed" : ""
                }`}
                style={{ fontSize: 13 }}
              >
                创建
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
