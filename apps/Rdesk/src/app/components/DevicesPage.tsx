import { useState, useEffect, useRef } from "react";
import { useNavigate } from "react-router";
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
import { NetworkGroupSelector } from "./NetworkGroupSelector";
import { NetworkGroupEditModal } from "./NetworkGroupEditModal";
import { useNetworkGroups } from "../hooks/useNetworkGroups";
import { networkGroupService } from "../services/networkGroupService";
import {
  launchLocalRemoteDisplayTest,
  launchRemoteDisplayForDevice,
} from "../services/remoteDisplayLauncher";

export function DevicesPage() {
  const { isDark } = useTheme();
  const navigate = useNavigate();
  const { devices, loading, error, refresh, lastUpdated, currentDeviceId } = useDevices({
    pollInterval: 30000,
    enabled: true,
  });
  const { groups, refresh: refreshGroups } = useNetworkGroups({ pollInterval: 30000, enabled: true });

  const [refreshing, setRefreshing] = useState(false);
  const [search, setSearch] = useState("");
  const [selectedDevice, setSelectedDevice] = useState<string | null>(null);
  const [menuOpen, setMenuOpen] = useState<string | null>(null);
  const [selectedGroupId, setSelectedGroupId] = useState<string | null>(null);
  const [editModalOpen, setEditModalOpen] = useState(false);
  const [editingGroupId, setEditingGroupId] = useState<string | null>(null);
  const [createGroupModalOpen, setCreateGroupModalOpen] = useState(false);
  const [newGroupName, setNewGroupName] = useState("");
  const [newGroupDescription, setNewGroupDescription] = useState("");
  const [launchingDeviceId, setLaunchingDeviceId] = useState<string | null>(null);

  // 右键菜单状态
  const [contextMenu, setContextMenu] = useState<{
    deviceId: string;
    x: number;
    y: number;
  } | null>(null);
  const [submenuOpen, setSubmenuOpen] = useState<string | null>(null);
  const contextMenuRef = useRef<HTMLDivElement>(null);

  // 默认选择第一个分组
  useEffect(() => {
    const firstGroup = groups[0];
    if (firstGroup && !selectedGroupId) {
      setSelectedGroupId(firstGroup.id);
    }
  }, [groups, selectedGroupId]);

  // 点击外部关闭右键菜单
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (contextMenuRef.current && !contextMenuRef.current.contains(e.target as Node)) {
        setContextMenu(null);
        setSubmenuOpen(null);
      }
    };
    if (contextMenu) {
      document.addEventListener("mousedown", handleClickOutside);
      return () => document.removeEventListener("mousedown", handleClickOutside);
    }
  }, [contextMenu]);

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      await Promise.all([refresh(), refreshGroups()]);
    } finally {
      setTimeout(() => setRefreshing(false), 500);
    }
  };

  // 根据选中的分组筛选设备
  const filteredDevices = devices.filter((d) => {
    const matchSearch =
      d.name.toLowerCase().includes(search.toLowerCase()) ||
      d.deviceId.includes(search) ||
      d.os.toLowerCase().includes(search.toLowerCase());

    // TODO: 这里需要根据实际分组关系筛选，暂时返回所有设备
    const matchGroup = true;

    return matchSearch && matchGroup;
  });

  const online = devices.filter((d) => d.status === "online").length;

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

  // 当前选中的分组
  const currentGroup = groups.find((g) => g.id === selectedGroupId) || null;

  // 右键菜单处理
  const handleContextMenu = (e: React.MouseEvent, deviceId: string) => {
    e.preventDefault();
    e.stopPropagation();
    setContextMenu({ deviceId, x: e.clientX, y: e.clientY });
  };

  const contextMenuDevice = contextMenu ? devices.find((d) => d.id === contextMenu.deviceId) : null;
  const isContextOnline = contextMenuDevice?.status === "online";

  // 模拟管理操作
  const handleManagementAction = (action: string) => {
    console.log(`执行管理操作: ${action} on ${contextMenu?.deviceId}`);
    setContextMenu(null);
    setSubmenuOpen(null);
    // TODO: 实际实现这些操作
    alert(`${action} 功能为模拟操作`);
  };

  // 打开编辑弹窗
  const handleOpenEditModal = (groupId: string | null) => {
    setEditingGroupId(groupId);
    setEditModalOpen(true);
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

  const handleOpenRemoteWindow = async (device: Device) => {
    setLaunchingDeviceId(device.id);
    try {
      const result = await launchRemoteDisplayForDevice(device.deviceId, {
        transportKind: device.p2pAvailable
          ? "quic"
          : device.os.toLowerCase().includes("quic")
            ? "quic"
            : "webrtc",
        targetDeviceName: device.name,
        targetOs: device.os,
        targetIp: device.ip,
        lanP2P: device.p2pAvailable && !device.isLocal,
      });
      if (result.mode === "route") navigate(`/session/${result.sessionId}`);
    } catch (error) {
      alert(error instanceof Error ? error.message : "Open remote display failed");
    } finally {
      setLaunchingDeviceId(null);
      setContextMenu(null);
    }
  };

  const handleOpenLocalTestWindow = async () => {
    setLaunchingDeviceId("__local_test__");
    try {
      const result = await launchLocalRemoteDisplayTest();
      if (result.mode === "route") navigate(`/session/${result.sessionId}`);
    } catch (error) {
      alert(error instanceof Error ? error.message : "Open local test display failed");
    } finally {
      setLaunchingDeviceId(null);
    }
  };

  if (loading) {
    return (
      <div className={`flex items-center justify-center h-full ${textPrimary}`}>
        加载设备中...
      </div>
    );
  }
  if (error) {
    return (
      <div className={`flex items-center justify-center h-full text-red-500`}>
        设备加载失败: {error}
      </div>
    );
  }

  return (
    <div className="flex h-full">
      {/* Main list */}
      <div className="flex-1 p-8 overflow-y-auto">
        {/* Toolbar */}
        <div className="flex items-center gap-3 mb-5">
          {/* 搜索框 */}
          <div className="relative flex-1 max-w-xs">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400" />
            <input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="搜索设备..."
              className={`w-full pl-9 pr-3 py-1.5 rounded-lg border outline-none transition-all ${inputBg}`}
              style={{ fontSize: 13 }}
            />
          </div>

          {/* 网络分组选择器 - 只有有分组时显示 */}
          {groups.length > 0 && (
            <NetworkGroupSelector
              groups={groups}
              selectedGroupId={selectedGroupId}
              onSelectGroup={setSelectedGroupId}
              onCreateGroup={handleOpenCreateModal}
              isDark={isDark}
            />
          )}

          {/* 编辑按钮 - 只有选中分组时显示 */}
          {selectedGroupId && (
            <button
              onClick={() => handleOpenEditModal(selectedGroupId)}
              className={`flex items-center gap-2 px-3 py-1.5 rounded-lg border transition-colors ${
                isDark
                  ? "bg-[#232323] border-gray-600 text-gray-300 hover:border-gray-500"
                  : "bg-white border-gray-200 text-gray-700 hover:border-gray-300"
              }`}
              style={{ fontSize: 13 }}
            >
              <Edit2 className="w-3.5 h-3.5" />
              编辑
            </button>
          )}

          {/* 添加分组按钮 - 没有分组时显示 */}
          {groups.length === 0 && (
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
          <button
            onClick={handleOpenLocalTestWindow}
            disabled={launchingDeviceId === "__local_test__"}
            className={`flex items-center gap-2 px-3.5 py-1.5 rounded-lg border transition-colors ${
              isDark
                ? "bg-[#232323] border-gray-600 text-gray-200 hover:border-gray-500"
                : "bg-white border-gray-200 text-gray-700 hover:border-gray-300"
            } disabled:cursor-not-allowed disabled:opacity-60`}
            style={{ fontSize: 13 }}
          >
            <Monitor className="w-3.5 h-3.5" />
            {launchingDeviceId === "__local_test__" ? "Opening..." : "本机测试窗口"}
          </button>
          <button
            className="flex items-center gap-2 px-3.5 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-white transition-colors shadow-sm"
            style={{ fontSize: 13 }}
          >
            <Plus className="w-3.5 h-3.5" />
            添加设备
          </button>
        </div>

        {/* 设备列表 */}
        <div className="grid grid-cols-2 gap-3">
          {filteredDevices.map((device) => {
            const Icon = device.icon;
            const isSelected = selectedDevice === device.id;
            const isCurrentDevice = device.deviceId === currentDeviceId;
            return (
              <div
                key={device.id}
                onContextMenu={(e) => handleContextMenu(e, device.id)}
                onClick={() => navigate(`/devices/${device.id}`)}
                className={`relative p-4 rounded-xl border cursor-pointer transition-all ${
                  isSelected
                    ? isDark
                      ? "bg-blue-900/20 border-blue-700 shadow-sm"
                      : "bg-blue-50/70 border-blue-300 shadow-sm"
                    : `${card} ${cardHover}`
                }`}
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

                  <div className="relative">
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        setMenuOpen(menuOpen === device.id ? null : device.id);
                      }}
                      className={`p-1 rounded-md ${
                        isDark
                          ? "text-gray-500 hover:text-gray-300 hover:bg-gray-700"
                          : "text-gray-400 hover:text-gray-600 hover:bg-gray-100"
                      }`}
                    >
                      <MoreVertical className="w-4 h-4" />
                    </button>
                    {menuOpen === device.id && (
                      <div
                        className={`absolute right-0 top-7 w-36 py-1 rounded-lg border shadow-lg z-10 ${
                          isDark ? "bg-[#2a2a2a] border-gray-600" : "bg-white border-gray-200"
                        }`}
                      >
                        {[
                          { icon: Edit2, label: "重命名" },
                          { icon: Star, label: "收藏" },
                          { icon: Trash2, label: "删除", danger: true },
                        ].map(({ icon: I, label, danger }) => (
                          <button
                            key={label}
                            onClick={(e) => {
                              e.stopPropagation();
                              setMenuOpen(null);
                            }}
                            className={`flex items-center gap-2 w-full px-3 py-2 text-left transition-colors ${
                              danger
                                ? "text-red-500"
                                : isDark
                                  ? "text-gray-300 hover:bg-gray-700"
                                  : "text-gray-600 hover:bg-gray-50"
                            }`}
                            style={{ fontSize: 13 }}
                          >
                            <I className="w-3.5 h-3.5" />
                            {label}
                          </button>
                        ))}
                      </div>
                    )}
                  </div>
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

                {device.status === "online" ? (
                  <>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        void handleOpenRemoteWindow(device);
                      }}
                      disabled={launchingDeviceId === device.id}
                      className={`w-full mb-2 py-2 rounded-lg transition-colors flex items-center justify-center gap-1.5 ${
                        isDark
                          ? "bg-emerald-900/30 hover:bg-emerald-900/50 text-emerald-300"
                          : "bg-emerald-50 hover:bg-emerald-100 text-emerald-700"
                      } disabled:cursor-not-allowed disabled:opacity-60`}
                      style={{ fontSize: 13 }}
                    >
                      <Monitor className="w-3.5 h-3.5" />
                      {launchingDeviceId === device.id ? "Opening..." : "P2P 连接"}
                    </button>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      navigate(`/devices/${device.id}`);
                    }}
                    className={`w-full py-2 rounded-lg transition-colors flex items-center justify-center gap-1.5 ${
                      isDark
                        ? "bg-blue-900/30 hover:bg-blue-900/50 text-blue-400"
                        : "bg-blue-50 hover:bg-blue-100 text-blue-600"
                    }`}
                    style={{ fontSize: 13 }}
                  >
                    <ExternalLink className="w-3.5 h-3.5" />
                    查看详情
                  </button>
                  </>
                ) : (
                  <div
                    className={`w-full py-2 rounded-lg text-center flex items-center justify-center gap-1.5 ${
                      isDark ? "bg-gray-800 text-gray-500" : "bg-gray-50 text-gray-400"
                    }`}
                    style={{ fontSize: 13 }}
                  >
                    <WifiOff className="w-3.5 h-3.5" />
                    设备离线
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {/* 右键菜单 */}
      {contextMenu && contextMenuDevice && (
        <div
          ref={contextMenuRef}
          className={`fixed z-50 min-w-[160px] rounded-lg border py-1 shadow-xl ${
            isDark ? "bg-[#2a2a2a] border-gray-700" : "bg-white border-gray-200"
          }`}
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onMouseLeave={() => setSubmenuOpen(null)}
        >
          {/* 在线设备特有菜单 */}
          {isContextOnline && (
            <>
              <button
                onClick={() => {
                  void handleOpenRemoteWindow(contextMenuDevice);
                }}
                className={`w-full flex items-center gap-2.5 px-3 py-1.5 text-left transition-colors ${
                  isDark ? "text-gray-300 hover:bg-gray-700" : "text-gray-600 hover:bg-gray-50"
                }`}
                style={{ fontSize: 12 }}
              >
                <ExternalLink className="w-4 h-4" />
                <span>远程桌面</span>
              </button>
              <div className={`h-px my-1 mx-2 ${isDark ? "bg-gray-700" : "bg-gray-100"}`} />
            </>
          )}

          {/* 通用菜单 */}
          <button
            onClick={() => {
              // TODO: 实现重命名
              setContextMenu(null);
            }}
            className={`w-full flex items-center gap-2.5 px-3 py-1.5 text-left transition-colors ${
              isDark ? "text-gray-300 hover:bg-gray-700" : "text-gray-600 hover:bg-gray-50"
            }`}
            style={{ fontSize: 12 }}
          >
            <Edit2 className="w-4 h-4" />
            <span>重命名</span>
          </button>
          <button
            onClick={() => setContextMenu(null)}
            className={`w-full flex items-center gap-2.5 px-3 py-1.5 text-left transition-colors ${
              isDark ? "text-gray-300 hover:bg-gray-700" : "text-gray-600 hover:bg-gray-50"
            }`}
            style={{ fontSize: 12 }}
          >
            <Star className="w-4 h-4" />
            <span>收藏设备</span>
          </button>
          <div className={`h-px my-1 mx-2 ${isDark ? "bg-gray-700" : "bg-gray-100"}`} />

          {/* 启用/禁用 */}
          <button
            onClick={() => handleManagementAction("toggle_device")}
            className={`w-full flex items-center gap-2.5 px-3 py-1.5 text-left transition-colors ${
              isDark ? "text-gray-300 hover:bg-gray-700" : "text-gray-600 hover:bg-gray-50"
            }`}
            style={{ fontSize: 12 }}
          >
            <Power className="w-4 h-4" />
            <span>禁用设备</span>
          </button>

          <div className={`h-px my-1 mx-2 ${isDark ? "bg-gray-700" : "bg-gray-100"}`} />

          {/* 移除设备和管理子菜单 */}
          <button
            onClick={() => setContextMenu(null)}
            className={`w-full flex items-center gap-2.5 px-3 py-1.5 text-left transition-colors ${
              isDark ? "text-red-400 hover:bg-red-900/30" : "text-red-500 hover:bg-red-50"
            }`}
            style={{ fontSize: 12 }}
          >
            <Trash2 className="w-4 h-4" />
            <span>移除设备</span>
          </button>

          {/* 管理子菜单 */}
          <div className="relative">
            <button
              onMouseEnter={() => setSubmenuOpen("management")}
              className={`w-full flex items-center justify-between gap-2.5 px-3 py-1.5 text-left transition-colors ${
                isDark ? "text-gray-300 hover:bg-gray-700" : "text-gray-600 hover:bg-gray-50"
              }`}
              style={{ fontSize: 12 }}
            >
              <span className="flex items-center gap-2.5">
                <Power className="w-4 h-4" />
                <span>管理</span>
              </span>
              <span className="text-xs">›</span>
            </button>

            {submenuOpen === "management" && (
              <div
                className={`absolute left-full top-0 ml-1 min-w-[140px] rounded-lg border py-1 shadow-xl ${
                  isDark ? "bg-[#2a2a2a] border-gray-700" : "bg-white border-gray-200"
                }`}
                onMouseEnter={() => setSubmenuOpen("management")}
              >
                {[
                  { icon: RotateCw, label: "重启", action: "restart" },
                  { icon: Power, label: "关机", action: "shutdown" },
                  { icon: Zap, label: "Wake-on-LAN", action: "wol" },
                  { icon: Info, label: "设备信息", action: "info" },
                ].map((item) => {
                  const Icon = item.icon;
                  return (
                    <button
                      key={item.label}
                      onClick={() => handleManagementAction(item.action)}
                      className={`w-full flex items-center gap-2.5 px-3 py-1.5 text-left transition-colors ${
                        isDark ? "text-gray-300 hover:bg-gray-700" : "text-gray-600 hover:bg-gray-50"
                      }`}
                      style={{ fontSize: 12 }}
                    >
                      <Icon className="w-4 h-4" />
                      <span>{item.label}</span>
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      )}

      {/* 编辑弹窗 */}
      {editModalOpen && (
        <NetworkGroupEditModal
          group={groups.find((g) => g.id === editingGroupId) || null}
          allDevices={devices.map((d) => ({
            id: d.id,
            name: d.name,
            device_id: d.deviceId,
            status: d.status,
            ip: d.ip,
          }))}
          isOpen={editModalOpen}
          onClose={() => setEditModalOpen(false)}
          onSave={async () => {
            await refreshGroups();
            setEditModalOpen(false);
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
