import { useCallback, useEffect, useState } from "react";
import { networkGroupService, NetworkGroup } from "../services/networkGroupService";
import { useAuth } from "../components/AuthContext";

/**
 * 网络分组数据 Hook
 *
 * 提供网络分组列表的加载、刷新和状态管理。
 */
export interface UseNetworkGroupsOptions {
  pollInterval?: number;      // 轮询间隔（毫秒），默认 30000
  enabled?: boolean;          // 是否启用轮询，默认 true
}

export function useNetworkGroups(options?: UseNetworkGroupsOptions) {
  const [groups, setGroups] = useState<NetworkGroup[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const { isLoggedIn, token } = useAuth();

  const {
    pollInterval = 30000,
    enabled = true,
  } = options || {};

  const fetchGroups = useCallback(async () => {
    // 只有登录后才加载分组列表
    if (!isLoggedIn || !token) {
      setGroups([]);
      setLoading(false);
      setError(null);
      return;
    }

    setLoading(true);
    setError(null);
    try {
      const data = await networkGroupService.getNetworkGroups();
      setGroups(data);
      setLastUpdated(new Date());
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载网络分组失败");
    } finally {
      setLoading(false);
    }
  }, [isLoggedIn, token]);

  // 初始加载
  useEffect(() => {
    if (isLoggedIn && token) {
      fetchGroups();
    } else {
      setGroups([]);
      setLoading(false);
    }
  }, [fetchGroups, isLoggedIn, token]);

  // 轮询
  useEffect(() => {
    if (!enabled) return;
    const interval = setInterval(() => {
      if (!document.hidden) fetchGroups();
    }, pollInterval);
    return () => clearInterval(interval);
  }, [fetchGroups, pollInterval, enabled]);

  // 页面恢复时刷新
  useEffect(() => {
    if (!enabled) return;
    const handleVisibilityChange = () => {
      if (!document.hidden) fetchGroups();
    };
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => document.removeEventListener("visibilitychange", handleVisibilityChange);
  }, [fetchGroups, enabled]);

  return {
    groups,
    loading,
    error,
    lastUpdated,
    refresh: fetchGroups,
  };
}

/**
 * 获取指定 ID 的网络分组
 */
export function useNetworkGroupById(id: string | undefined, groups: NetworkGroup[]) {
  return groups.find((g) => g.id === id);
}
