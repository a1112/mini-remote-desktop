import { useState, useEffect } from "react";
import {
  NetworkGroup,
  DeviceInGroup,
  networkGroupService,
} from "../services/networkGroupService";
import {
  X,
  Info,
  Monitor,
  ChevronLeft,
  ChevronRight,
  Plus,
  Minus,
  Trash2,
  Settings,
  Globe,
  Check,
} from "lucide-react";

interface Device {
  id: string;
  name: string;
  device_id: string;
  status: "online" | "offline";
  ip: string;
}

interface NetworkGroupEditModalProps {
  group: NetworkGroup | null;
  allDevices: Device[];
  isOpen: boolean;
  onClose: () => void;
  onSave: () => void;
  isDark?: boolean;
}

type Tab = "info" | "devices" | "settings";

/**
 * 网络分组编辑弹窗
 *
 * 支持编辑分组信息、管理分组设备、配置组网设置。
 */
export function NetworkGroupEditModal({
  group,
  allDevices,
  isOpen,
  onClose,
  onSave,
  isDark = true,
}: NetworkGroupEditModalProps) {
  const [activeTab, setActiveTab] = useState<Tab>("info");
  const [groupDevices, setGroupDevices] = useState<DeviceInGroup[]>([]);
  const [groupName, setGroupName] = useState("");
  const [groupDescription, setGroupDescription] = useState("");
  const [isGroupEnabled, setIsGroupEnabled] = useState(true);
  const [selectedDeviceIds, setSelectedDeviceIds] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);

  // 当弹窗打开时加载数据
  useEffect(() => {
    if (isOpen && group) {
      setActiveTab("info");
      setGroupName(group.name);
      setGroupDescription(group.description || "");
      setIsGroupEnabled(group.is_enabled);
      setSelectedDeviceIds([]);
      loadGroupDevices();
    }
  }, [isOpen, group]);

  const loadGroupDevices = async () => {
    if (!group) return;
    setLoading(true);
    try {
      const devices = await networkGroupService.getGroupDevices(group.id);
      setGroupDevices(devices);
    } catch (e) {
      console.error("加载分组设备失败:", e);
    } finally {
      setLoading(false);
    }
  };

  // 获取不在当前分组中的设备
  const availableDevices = allDevices.filter(
    (d) => !groupDevices.some((gd) => gd.device_id === d.device_id)
  );

  // 添加设备到分组
  const handleAddDevices = async () => {
    if (!group || selectedDeviceIds.length === 0) return;

    setSaving(true);
    setMessage(null);
    try {
      await networkGroupService.addDevicesToGroup(group.id, selectedDeviceIds);
      await loadGroupDevices();
      setSelectedDeviceIds([]);
      setMessage({ type: "success", text: "设备添加成功" });
    } catch (e) {
      setMessage({ type: "error", text: "添加设备失败" });
    } finally {
      setSaving(false);
    }
  };

  // 从分组移除设备
  const handleRemoveDevice = async (device: DeviceInGroup) => {
    if (!group) return;

    setSaving(true);
    setMessage(null);
    try {
      await networkGroupService.removeDeviceFromGroup(group.id, device.device_id);
      await loadGroupDevices();
      setMessage({ type: "success", text: "设备移除成功" });
    } catch (e) {
      setMessage({ type: "error", text: "移除设备失败" });
    } finally {
      setSaving(false);
    }
  };

  // 切换设备启用状态
  const handleToggleDeviceEnabled = async (device: DeviceInGroup) => {
    if (!group) return;

    setSaving(true);
    setMessage(null);
    try {
      await networkGroupService.setDeviceEnabled(
        group.id,
        device.device_id,
        !device.is_enabled
      );
      await loadGroupDevices();
      setMessage({ type: "success", text: "设备状态更新成功" });
    } catch (e) {
      setMessage({ type: "error", text: "更新状态失败" });
    } finally {
      setSaving(false);
    }
  };

  // 保存分组信息
  const handleSaveGroupInfo = async () => {
    if (!group) return;

    setSaving(true);
    setMessage(null);
    try {
      await networkGroupService.updateNetworkGroup(group.id, {
        name: groupName,
        description: groupDescription,
        is_enabled: isGroupEnabled,
      });
      setMessage({ type: "success", text: "分组信息更新成功" });
      onSave();
    } catch (e) {
      setMessage({ type: "error", text: "更新失败" });
    } finally {
      setSaving(false);
    }
  };

  // 删除分组
  const handleDeleteGroup = async () => {
    if (!group || group.name === "默认网络") return;

    if (!confirm(`确定要删除分组"${group.name}"吗？此操作不可恢复。`)) {
      return;
    }

    setSaving(true);
    setMessage(null);
    try {
      await networkGroupService.deleteNetworkGroup(group.id);
      onSave();
      onClose();
    } catch (e) {
      setMessage({ type: "error", text: "删除失败" });
    } finally {
      setSaving(false);
    }
  };

  // 选择/取消选择设备
  const toggleDeviceSelection = (deviceId: string) => {
    setSelectedDeviceIds((prev) =>
      prev.includes(deviceId)
        ? prev.filter((id) => id !== deviceId)
        : [...prev, deviceId]
    );
  };

  if (!isOpen || !group) return null;

  const cardBg = isDark ? "bg-[#232323]" : "bg-white";
  const border = isDark ? "border-gray-700" : "border-gray-200";
  const textPrimary = isDark ? "text-gray-100" : "text-gray-900";
  const textSecondary = isDark ? "text-gray-400" : "text-gray-500";
  const inputBg = isDark
    ? "bg-[#2a2a2a] border-gray-600 text-gray-200"
    : "bg-gray-50 border-gray-200 text-gray-900";
  const tabActive = isDark ? "text-blue-400 border-blue-400" : "text-blue-600 border-blue-600";
  const tabInactive = isDark
    ? "text-gray-400 border-transparent hover:text-gray-200"
    : "text-gray-500 border-transparent hover:text-gray-700";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div
        className={`w-[600px] max-h-[80vh] rounded-xl border shadow-2xl flex flex-col ${cardBg} ${border}`}
      >
        {/* 标题栏 */}
        <div className={`flex items-center justify-between px-5 py-4 border-b ${border}`}>
          <h2 className={`text-lg font-semibold ${textPrimary}`}>编辑网络分组</h2>
          <button
            onClick={onClose}
            className={`p-1 rounded-md transition-colors ${
              isDark ? "hover:bg-gray-700 text-gray-400" : "hover:bg-gray-100 text-gray-500"
            }`}
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* 标签页切换 */}
        <div className={`flex items-center gap-6 px-5 py-3 border-b ${border}`}>
          {[
            { key: "info" as Tab, label: "分组信息", icon: Info },
            { key: "devices" as Tab, label: "设备管理", icon: Monitor },
            { key: "settings" as Tab, label: "组网设置", icon: Settings },
          ].map((tab) => {
            const Icon = tab.icon;
            const isActive = activeTab === tab.key;
            return (
              <button
                key={tab.key}
                onClick={() => setActiveTab(tab.key)}
                className={`flex items-center gap-2 pb-2 border-b-2 transition-colors ${
                  isActive ? tabActive : tabInactive
                }`}
                style={{ fontSize: 13 }}
              >
                <Icon className="w-4 h-4" />
                {tab.label}
              </button>
            );
          })}
        </div>

        {/* 内容区域 */}
        <div className="flex-1 overflow-y-auto px-5 py-4">
          {/* 分组信息 */}
          {activeTab === "info" && (
            <div className="space-y-4">
              <div>
                <label className={`block mb-2 ${textSecondary}`} style={{ fontSize: 12 }}>
                  分组名称
                </label>
                <input
                  value={groupName}
                  onChange={(e) => setGroupName(e.target.value)}
                  className={`w-full px-3 py-2 rounded-lg border outline-none ${inputBg}`}
                  style={{ fontSize: 13 }}
                />
              </div>
              <div>
                <label className={`block mb-2 ${textSecondary}`} style={{ fontSize: 12 }}>
                  分组描述
                </label>
                <textarea
                  value={groupDescription}
                  onChange={(e) => setGroupDescription(e.target.value)}
                  className={`w-full px-3 py-2 rounded-lg border outline-none ${inputBg}`}
                  rows={3}
                  style={{ fontSize: 13 }}
                />
              </div>

              {/* 统计信息 */}
              <div className={`p-4 rounded-lg border ${isDark ? "bg-[#2a2a2a] border-gray-600" : "bg-gray-50 border-gray-200"}`}>
                <div className="flex items-center justify-between mb-2">
                  <span className={textSecondary} style={{ fontSize: 12 }}>设备总数</span>
                  <span className={`font-semibold ${textPrimary}`} style={{ fontSize: 14 }}>
                    {group.device_count}
                  </span>
                </div>
                <div className="flex items-center justify-between">
                  <span className={textSecondary} style={{ fontSize: 12 }}>在线设备</span>
                  <span className={`font-semibold text-green-500`} style={{ fontSize: 14 }}>
                    {group.online_device_count}
                  </span>
                </div>
              </div>

              {/* 启用开关 */}
              <div className="flex items-center justify-between">
                <div>
                  <div className={textPrimary} style={{ fontSize: 13 }}>启用分组</div>
                  <div className={textSecondary} style={{ fontSize: 11 }}>
                    禁用后分组内设备将不可用
                  </div>
                </div>
                <button
                  onClick={() => setIsGroupEnabled(!isGroupEnabled)}
                  className={`w-11 h-6 rounded-full relative transition-colors ${
                    isGroupEnabled ? "bg-blue-600" : isDark ? "bg-gray-600" : "bg-gray-300"
                  }`}
                >
                  <div
                    className={`absolute top-0.5 w-5 h-5 rounded-full bg-white shadow-sm transition-transform ${
                      isGroupEnabled ? "left-[22px]" : "left-0.5"
                    }`}
                  />
                </button>
              </div>

              {/* 删除按钮 */}
              {group.name !== "默认网络" && (
                <button
                  onClick={handleDeleteGroup}
                  disabled={saving}
                  className={`w-full flex items-center justify-center gap-2 px-4 py-2 rounded-lg border transition-colors ${
                    isDark
                      ? "border-red-900/50 text-red-400 hover:bg-red-900/20"
                      : "border-red-200 text-red-500 hover:bg-red-50"
                  }`}
                  style={{ fontSize: 13 }}
                >
                  <Trash2 className="w-4 h-4" />
                  删除分组
                </button>
              )}
            </div>
          )}

          {/* 设备管理 */}
          {activeTab === "devices" && (
            <div className="flex gap-4 h-full">
              {/* 全部设备 */}
              <div className="flex-1 flex flex-col">
                <div className={`flex items-center justify-between mb-2`}>
                  <span className={`font-medium ${textPrimary}`} style={{ fontSize: 13 }}>
                    全部设备 ({availableDevices.length})
                  </span>
                </div>
                <div className={`flex-1 overflow-y-auto rounded-lg border p-2 ${isDark ? "border-gray-700 bg-[#2a2a2a]" : "border-gray-200 bg-gray-50"}`}>
                  {availableDevices.length === 0 ? (
                    <div className={`text-center py-8 ${textSecondary}`} style={{ fontSize: 12 }}>
                      没有可用设备
                    </div>
                  ) : (
                    <div className="space-y-1">
                      {availableDevices.map((device) => {
                        const isSelected = selectedDeviceIds.includes(device.device_id);
                        return (
                          <div
                            key={device.id}
                            onClick={() => toggleDeviceSelection(device.device_id)}
                            className={`flex items-center gap-2 px-2 py-2 rounded cursor-pointer transition-colors ${
                              isSelected
                                ? isDark
                                  ? "bg-blue-900/30"
                                  : "bg-blue-50"
                                : isDark
                                  ? "hover:bg-gray-700"
                                  : "hover:bg-gray-100"
                            }`}
                          >
                            <input
                              type="checkbox"
                              checked={isSelected}
                              onChange={() => toggleDeviceSelection(device.device_id)}
                              className="shrink-0"
                            />
                            <Monitor
                              className={`w-4 h-4 shrink-0 ${
                                device.status === "online"
                                  ? "text-green-500"
                                  : "text-gray-400"
                              }`}
                            />
                            <span className={`flex-1 truncate ${textPrimary}`} style={{ fontSize: 12 }}>
                              {device.name}
                            </span>
                            {device.status === "online" && (
                              <span
                                className={`px-1.5 py-0.5 rounded text-[10px] ${
                                  isDark ? "bg-green-900/50 text-green-400" : "bg-green-100 text-green-600"
                                }`}
                              >
                                在线
                              </span>
                            )}
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
                {selectedDeviceIds.length > 0 && (
                  <button
                    onClick={handleAddDevices}
                    disabled={saving}
                    className={`mt-2 w-full flex items-center justify-center gap-2 px-3 py-2 rounded-lg bg-blue-600 hover:bg-blue-500 text-white transition-colors`}
                    style={{ fontSize: 12 }}
                  >
                    <Plus className="w-3.5 h-3.5" />
                    添加选中的设备 ({selectedDeviceIds.length})
                  </button>
                )}
              </div>

              {/* 分组设备 */}
              <div className="flex-1 flex flex-col">
                <div className={`flex items-center justify-between mb-2`}>
                  <span className={`font-medium ${textPrimary}`} style={{ fontSize: 13 }}>
                    分组设备 ({groupDevices.length})
                  </span>
                </div>
                <div className={`flex-1 overflow-y-auto rounded-lg border p-2 ${isDark ? "border-gray-700 bg-[#2a2a2a]" : "border-gray-200 bg-gray-50"}`}>
                  {loading ? (
                    <div className={`text-center py-8 ${textSecondary}`} style={{ fontSize: 12 }}>
                      加载中...
                    </div>
                  ) : groupDevices.length === 0 ? (
                    <div className={`text-center py-8 ${textSecondary}`} style={{ fontSize: 12 }}>
                      分组暂无设备
                    </div>
                  ) : (
                    <div className="space-y-1">
                      {groupDevices.map((device) => (
                        <div
                          key={device.id}
                          className={`flex items-center gap-2 px-2 py-2 rounded ${
                            isDark ? "bg-gray-800" : "bg-white"
                          }`}
                        >
                          <Monitor
                            className={`w-4 h-4 shrink-0 ${
                              device.status === "online"
                                ? "text-green-500"
                                : "text-gray-400"
                            }`}
                          />
                          <div className="flex-1 min-w-0">
                            <div className={`truncate ${textPrimary}`} style={{ fontSize: 12 }}>
                              {device.name}
                            </div>
                            <div className={`truncate ${textSecondary}`} style={{ fontSize: 10 }}>
                              {device.ip}
                            </div>
                          </div>
                          <button
                            onClick={() => handleToggleDeviceEnabled(device)}
                            className={`shrink-0 px-1.5 py-0.5 rounded text-[10px] transition-colors ${
                              device.is_enabled
                                ? isDark
                                  ? "bg-green-900/50 text-green-400"
                                  : "bg-green-100 text-green-600"
                                : isDark
                                  ? "bg-gray-700 text-gray-400"
                                  : "bg-gray-100 text-gray-500"
                            }`}
                          >
                            {device.is_enabled ? "已启用" : "已禁用"}
                          </button>
                          <button
                            onClick={() => handleRemoveDevice(device)}
                            className={`shrink-0 p-1 rounded transition-colors ${
                              isDark
                                ? "hover:bg-red-900/30 text-red-400"
                                : "hover:bg-red-50 text-red-500"
                            }`}
                          >
                            <Minus className="w-3 h-3" />
                          </button>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            </div>
          )}

          {/* 组网设置 */}
          {activeTab === "settings" && (
            <div className="space-y-4">
              <div className={`p-4 rounded-lg border ${isDark ? "bg-[#2a2a2a] border-gray-600" : "bg-gray-50 border-gray-200"}`}>
                <div className={`flex items-center gap-2 mb-3 ${textPrimary}`} style={{ fontSize: 13 }}>
                  <Globe className="w-4 h-4 text-blue-500" />
                  <span className="font-medium">网络协议</span>
                </div>
                <div className="space-y-3">
                  {[
                    { label: "WireGuard", desc: "高性能加密隧道", enabled: true },
                    { label: "QUIC", desc: "低延迟传输协议", enabled: true },
                    { label: "TCP 中继", desc: "NAT 穿透失败时回退", enabled: false },
                  ].map((proto) => (
                    <div key={proto.label} className="flex items-center justify-between">
                      <div>
                        <div className={textPrimary} style={{ fontSize: 12 }}>{proto.label}</div>
                        <div className={textSecondary} style={{ fontSize: 10 }}>{proto.desc}</div>
                      </div>
                      <div
                        className={`w-9 h-5 rounded-full relative cursor-pointer transition-colors ${
                          proto.enabled ? "bg-blue-600" : isDark ? "bg-gray-600" : "bg-gray-300"
                        }`}
                      >
                        <div
                          className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow-sm transition-transform ${
                            proto.enabled ? "left-[18px]" : "left-0.5"
                          }`}
                        />
                      </div>
                    </div>
                  ))}
                </div>
              </div>

              <div className={`p-4 rounded-lg border ${isDark ? "bg-[#2a2a2a] border-gray-600" : "bg-gray-50 border-gray-200"}`}>
                <div className={`font-medium mb-3 ${textPrimary}`} style={{ fontSize: 13 }}>
                  DNS 设置
                </div>
                <div className="space-y-2">
                  <input
                    defaultValue="10.0.1.1"
                    placeholder="主 DNS"
                    className={`w-full px-3 py-2 rounded-lg border outline-none ${inputBg}`}
                    style={{ fontSize: 12 }}
                  />
                  <input
                    defaultValue="8.8.8.8"
                    placeholder="备用 DNS"
                    className={`w-full px-3 py-2 rounded-lg border outline-none ${inputBg}`}
                    style={{ fontSize: 12 }}
                  />
                </div>
              </div>

              <div className={`p-4 rounded-lg border ${isDark ? "bg-[#2a2a2a] border-gray-600" : "bg-gray-50 border-gray-200"}`}>
                <div className={`font-medium mb-3 ${textPrimary}`} style={{ fontSize: 13 }}>
                  安全策略
                </div>
                <div className="space-y-3">
                  {[
                    { label: "端到端加密", desc: "AES-256-GCM", enabled: true },
                    { label: "设备认证", desc: "双向 mTLS 验证", enabled: true },
                  ].map((item) => (
                    <div key={item.label} className="flex items-center justify-between">
                      <div>
                        <div className={textPrimary} style={{ fontSize: 12 }}>{item.label}</div>
                        <div className={textSecondary} style={{ fontSize: 10 }}>{item.desc}</div>
                      </div>
                      <div
                        className={`w-9 h-5 rounded-full relative cursor-pointer transition-colors ${
                          item.enabled ? "bg-blue-600" : isDark ? "bg-gray-600" : "bg-gray-300"
                        }`}
                      >
                        <div
                          className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow-sm transition-transform ${
                            item.enabled ? "left-[18px]" : "left-0.5"
                          }`}
                        />
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          )}
        </div>

        {/* 消息提示 */}
        {message && (
          <div
            className={`mx-5 mb-2 px-3 py-2 rounded-lg text-center ${
              message.type === "success"
                ? isDark
                  ? "bg-green-900/30 text-green-400"
                  : "bg-green-50 text-green-600"
                : isDark
                  ? "bg-red-900/30 text-red-400"
                  : "bg-red-50 text-red-600"
            }`}
            style={{ fontSize: 12 }}
          >
            {message.text}
          </div>
        )}

        {/* 底部按钮 */}
        <div className={`flex items-center justify-end gap-3 px-5 py-4 border-t ${border}`}>
          <button
            onClick={onClose}
            disabled={saving}
            className={`px-4 py-2 rounded-lg border transition-colors ${isDark ? "border-gray-600 text-gray-300 hover:bg-gray-700" : "border-gray-200 text-gray-600 hover:bg-gray-50"}`}
            style={{ fontSize: 13 }}
          >
            取消
          </button>
          <button
            onClick={activeTab === "info" ? handleSaveGroupInfo : onSave}
            disabled={saving}
            className={`px-4 py-2 rounded-lg bg-blue-600 hover:bg-blue-500 text-white transition-colors flex items-center gap-2`}
            style={{ fontSize: 13 }}
          >
            {saving ? (
              <>
                <div className="w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                保存中...
              </>
            ) : (
              <>
                {activeTab === "info" && <Check className="w-4 h-4" />}
                保存
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
