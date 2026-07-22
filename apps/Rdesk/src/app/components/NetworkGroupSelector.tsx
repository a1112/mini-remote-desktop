import { useState, useEffect, useRef } from "react";
import { NetworkGroup } from "../services/networkGroupService";
import { Globe, ChevronDown } from "lucide-react";

interface NetworkGroupSelectorProps {
  groups: NetworkGroup[];
  selectedGroupId: string | null;
  onSelectGroup: (groupId: string | null) => void;
  onCreateGroup?: () => void;
  isDark?: boolean;
  disabled?: boolean;
}

/**
 * 网络分组选择器下拉框组件
 *
 * 显示用户的网络分组列表，支持选择分组。
 */
export function NetworkGroupSelector({
  groups,
  selectedGroupId,
  onSelectGroup,
  onCreateGroup,
  isDark = true,
  disabled = false,
}: NetworkGroupSelectorProps) {
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  // 点击外部关闭下拉框
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  // 当前选中的分组
  const selectedGroup = groups.find((g) => g.id === selectedGroupId) || null;

  const buttonBg = isDark ? "bg-[#232323] border-gray-600" : "bg-white border-gray-200";
  const buttonHover = isDark ? "hover:border-gray-500" : "hover:border-gray-300";
  const dropdownBg = isDark ? "bg-[#2a2a2a] border-gray-600" : "bg-white border-gray-200";
  const textPrimary = isDark ? "text-gray-200" : "text-gray-800";
  const textSecondary = isDark ? "text-gray-400" : "text-gray-500";
  const itemHover = isDark ? "hover:bg-gray-700" : "hover:bg-gray-50";
  const selectedBg = isDark ? "bg-blue-900/30 text-blue-400" : "bg-blue-50 text-blue-600";
  const emptyText = isDark ? "text-gray-500" : "text-gray-400";

  return (
    <div ref={dropdownRef} className="relative">
      {/* 下拉按钮 */}
      <button
        onClick={() => !disabled && setIsOpen(!isOpen)}
        disabled={disabled}
        className={`flex items-center gap-2 px-3 py-1.5 rounded-lg border transition-colors ${
          buttonBg
        } ${disabled ? "opacity-50 cursor-not-allowed" : buttonHover}`}
        style={{ fontSize: 13 }}
      >
        <Globe className={`w-3.5 h-3.5 ${selectedGroup?.is_enabled ? "text-green-600" : "text-gray-400"}`} />
        <span className={`font-medium ${textPrimary}`}>
          {selectedGroup?.name || "选择网络分组"}
        </span>
        {selectedGroup && (
          <span className={`text-xs ${textSecondary}`}>({selectedGroup.device_count})</span>
        )}
        <ChevronDown
          className={`w-3.5 h-3.5 transition-transform ${textSecondary} ${
            isOpen ? "rotate-180" : ""
          }`}
        />
      </button>

      {/* 下拉菜单 */}
      {isOpen && (
        <div
          className={`absolute left-0 top-full mt-1 w-48 py-1 rounded-lg border shadow-lg z-50 ${
            dropdownBg
          }`}
        >
          {groups.length === 0 ? (
            <div className={`px-3 py-3 text-center ${emptyText}`} style={{ fontSize: 12 }}>
              暂无网络分组
            </div>
          ) : (
            groups.map((group) => (
              <button
                key={group.id}
                onClick={() => {
                  onSelectGroup(group.id);
                  setIsOpen(false);
                }}
                className={`flex items-center gap-2.5 w-full px-3 py-2 text-left transition-colors ${
                  selectedGroupId === group.id ? selectedBg : `${textPrimary} ${itemHover}`
                }`}
                style={{ fontSize: 12 }}
              >
                <Globe
                  className={`w-3.5 h-3.5 ${
                    group.is_enabled ? "text-green-600" : "text-gray-400"
                  }`}
                />
                <span className="flex-1 truncate">{group.name}</span>
                <span className={textSecondary}>({group.device_count})</span>
              </button>
            ))
          )}
        </div>
      )}
    </div>
  );
}
