import { Fragment, useEffect, useState } from "react";
import { useNavigate } from "react-router";
import { ChevronDown, LineChart, X } from "lucide-react";
import {
  testListRuns,
} from "../../adapters/tauri/commands";
import type { TestRun, RunStatus } from "../../adapters/tauri/types";
import { TestTelemetryPanel } from "./TestTelemetryPanel";

export function TestHistoryPage() {
  const navigate = useNavigate();
  const [runs, setRuns] = useState<TestRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<{
    scenario?: string;
    status?: string;
  }>({});
  const [modalRunId, setModalRunId] = useState<string | null>(null);
  const [expandedRunId, setExpandedRunId] = useState<string | null>(null);

  useEffect(() => {
    loadRuns();
  }, [filter]);

  const loadRuns = async () => {
    setLoading(true);
    try {
      const data = await testListRuns({
        scenarioId: filter.scenario,
        status: filter.status,
        limit: 50,
      });
      if (data.ok) {
        setRuns(data.value);
      }
    } catch (error) {
      console.error("Failed to load runs:", error);
    } finally {
      setLoading(false);
    }
  };

  const statusConfig: Record<RunStatus, { label: string; color: string }> = {
    queued: { label: "已排队", color: "bg-gray-100 text-gray-800 dark:bg-gray-700 dark:text-gray-300" },
    preparing: { label: "准备中", color: "bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400" },
    running: { label: "运行中", color: "bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400" },
    completed: { label: "已完成", color: "bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400" },
    failed: { label: "失败", color: "bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-400" },
    skipped: { label: "已跳过", color: "bg-slate-100 text-slate-800 dark:bg-slate-800 dark:text-slate-300" },
    cancelled: { label: "已取消", color: "bg-yellow-100 text-yellow-800 dark:bg-yellow-900/30 dark:text-yellow-400" },
  };

  return (
    <div className="p-6 max-w-6xl mx-auto">
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold">历史记录</h1>
          <p className="text-gray-500 text-sm mt-1">
            查看历史测试结果
          </p>
        </div>
        <button
          onClick={loadRuns}
          className="px-4 py-2 bg-blue-600 text-white rounded hover:bg-blue-700"
        >
          刷新
        </button>
      </div>

      {/* Filters */}
      <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-4 mb-6">
        <div className="flex gap-4">
          <div className="flex-1">
            <label className="block text-sm font-medium mb-1">场景</label>
            <select
              value={filter.scenario || "all"}
              onChange={(e) =>
                setFilter({ ...filter, scenario: e.target.value === "all" ? undefined : e.target.value })
              }
              className="w-full px-3 py-2 border rounded dark:bg-gray-700 dark:border-gray-600"
            >
              <option value="all">全部</option>
              <option value="e2e.local">端到端本地测试</option>
              <option value="encode.nvenc_h264">NVENC H.264 编码</option>
              <option value="encode.openh264">OpenH264 编码</option>
            </select>
          </div>
          <div className="flex-1">
            <label className="block text-sm font-medium mb-1">状态</label>
            <select
              value={filter.status || "all"}
              onChange={(e) =>
                setFilter({ ...filter, status: e.target.value === "all" ? undefined : e.target.value })
              }
              className="w-full px-3 py-2 border rounded dark:bg-gray-700 dark:border-gray-600"
            >
              <option value="all">全部</option>
              <option value="running">运行中</option>
              <option value="completed">已完成</option>
              <option value="failed">失败</option>
              <option value="skipped">已跳过</option>
              <option value="cancelled">已取消</option>
            </select>
          </div>
        </div>
      </div>

      {/* Runs List */}
      {loading ? (
        <div className="flex items-center justify-center h-64">
          <div className="h-6 w-6 animate-spin rounded-full border-2 border-muted border-t-primary" />
        </div>
      ) : runs.length === 0 ? (
        <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-8 text-center text-gray-500">
          <p>暂无测试记录</p>
        </div>
      ) : (
        <div className="bg-white dark:bg-gray-800 rounded-lg shadow overflow-hidden">
          <table className="w-full">
            <thead className="bg-gray-50 dark:bg-gray-700">
              <tr>
                <th className="px-4 py-3 text-left text-sm font-medium">状态</th>
                <th className="px-4 py-3 text-left text-sm font-medium">场景</th>
                <th className="px-4 py-3 text-left text-sm font-medium">开始时间</th>
                <th className="px-4 py-3 text-left text-sm font-medium">时长</th>
                <th className="px-4 py-3 text-left text-sm font-medium">结果</th>
                <th className="px-4 py-3 text-right text-sm font-medium">操作</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
              {runs.map((run) => {
                const statusInfo = statusConfig[run.status] ?? statusConfig.queued;
                const duration = ((run.finished_at || Date.now()) - run.started_at);
                const expanded = expandedRunId === run.run_id;

                return (
                  <Fragment key={run.run_id}>
                    <tr
                      className="hover:bg-gray-50 dark:hover:bg-gray-700/50 cursor-pointer"
                      onClick={() => navigate(`/test/run/${run.run_id}`)}
                    >
                      <td className="px-4 py-3">
                        <span className={`px-2 py-1 rounded-full text-xs font-medium ${statusInfo.color}`}>
                          {statusInfo.label}
                        </span>
                      </td>
                      <td className="px-4 py-3 text-sm font-mono">{run.scenario_id}</td>
                      <td className="px-4 py-3 text-sm">
                        {new Date(run.started_at).toLocaleString()}
                      </td>
                      <td className="px-4 py-3 text-sm">
                        {duration >= 60000
                          ? `${Math.floor(duration / 60000)}m ${Math.floor((duration % 60000) / 1000)}s`
                          : `${Math.floor(duration / 1000)}s`}
                      </td>
                      <td className="px-4 py-3 text-sm">
                        {run.summary ? (
                          <div className="text-xs">
                            <div>{run.summary.frame_count} 帧</div>
                            {run.summary.capture_fps && (
                              <div className="text-gray-500">{run.summary.capture_fps.toFixed(1)} FPS</div>
                            )}
                          </div>
                        ) : (
                          <span className="text-gray-400">-</span>
                        )}
                      </td>
                      <td className="px-4 py-3 text-right">
                        <div className="flex items-center justify-end gap-2">
                          <button
                            type="button"
                            onClick={(e) => {
                              e.stopPropagation();
                              setModalRunId(run.run_id);
                            }}
                            className="inline-flex items-center gap-1 text-sm font-medium text-blue-600 hover:text-blue-800"
                          >
                            <LineChart className="h-4 w-4" aria-hidden="true" />
                            曲线
                          </button>
                          <button
                            type="button"
                            aria-expanded={expanded}
                            onClick={(e) => {
                              e.stopPropagation();
                              setExpandedRunId(expanded ? null : run.run_id);
                            }}
                            className="inline-flex items-center gap-1 text-sm font-medium text-blue-600 hover:text-blue-800"
                          >
                            <ChevronDown
                              className={`h-4 w-4 transition-transform ${expanded ? "rotate-180" : ""}`}
                              aria-hidden="true"
                            />
                            展开
                          </button>
                          <button
                            onClick={(e) => {
                              e.stopPropagation();
                              navigate(`/test/run/${run.run_id}`);
                            }}
                            className="text-sm font-medium text-blue-600 hover:text-blue-800"
                          >
                            查看详情
                          </button>
                        </div>
                      </td>
                    </tr>
                    {expanded && (
                      <tr>
                        <td colSpan={6} className="bg-gray-50 p-4 dark:bg-gray-900/40">
                          <TestTelemetryPanel runId={run.run_id} mode="inline" />
                        </td>
                      </tr>
                    )}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      {modalRunId && (
        <div
          role="dialog"
          aria-modal="true"
          aria-label="测试曲线"
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
          onClick={() => setModalRunId(null)}
        >
          <div
            className="max-h-[90vh] w-full max-w-6xl overflow-auto rounded-lg bg-white p-4 shadow-xl dark:bg-gray-900"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="mb-3 flex justify-end">
              <button
                type="button"
                onClick={() => setModalRunId(null)}
                className="inline-flex h-9 w-9 items-center justify-center rounded hover:bg-gray-100 dark:hover:bg-gray-800"
                aria-label="关闭"
              >
                <X className="h-5 w-5" aria-hidden="true" />
              </button>
            </div>
            <TestTelemetryPanel runId={modalRunId} mode="modal" />
          </div>
        </div>
      )}
    </div>
  );
}
