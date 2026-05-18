import { useEffect, useState } from "react";
import { useParams, useNavigate } from "react-router";
import {
  testGetRun,
  testGetRunEvents,
  testStopRun,
} from "../../adapters/tauri/commands";
import type {
  TestRun,
  TestStageEvent,
} from "../../adapters/tauri/types";
import { TestTelemetryPanel } from "./TestTelemetryPanel";

export function RunDetailPage() {
  const { runId } = useParams<{ runId: string }>();
  const navigate = useNavigate();

  const [run, setRun] = useState<TestRun | null>(null);
  const [events, setEvents] = useState<TestStageEvent[]>([]);
  const [loading, setLoading] = useState(true);
  const [stopping, setStopping] = useState(false);

  // Load run data
  useEffect(() => {
    if (!runId) return;

    const loadData = async () => {
      try {
        const [runData, eventsData] = await Promise.all([
          testGetRun(runId),
          testGetRunEvents(runId),
        ]);

        if (!runData.ok || !runData.value) {
          navigate("/test/history");
          return;
        }

        setRun(runData.value);
        if (eventsData.ok) {
          setEvents(eventsData.value);
        }
      } catch (error) {
        console.error("Failed to load run data:", error);
      } finally {
        setLoading(false);
      }
    };

    loadData();
  }, [runId, navigate]);

  // Poll for updates when running
  useEffect(() => {
    if (!runId || run?.status !== "running") return;

    const interval = setInterval(async () => {
      try {
        const [runData, eventsData] = await Promise.all([
          testGetRun(runId),
          testGetRunEvents(runId),
        ]);

        if (!runData.ok || !runData.value) {
          return;
        }

        setRun(runData.value);
        if (eventsData.ok) {
          setEvents(eventsData.value);
        }

        if (runData.value.status !== "running") {
          clearInterval(interval);
        }
      } catch (error) {
        console.error("Failed to poll updates:", error);
      }
    }, 500);

    return () => clearInterval(interval);
  }, [runId, run?.status]);

  const handleStop = async () => {
    if (!runId || stopping) return;
    setStopping(true);
    try {
      await testStopRun(runId);
      // The poll will update the status
    } catch (error) {
      console.error("Failed to stop run:", error);
    } finally {
      setStopping(false);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center h-screen">
        <div className="h-6 w-6 animate-spin rounded-full border-2 border-muted border-t-primary" />
      </div>
    );
  }

  if (!run) {
    return (
      <div className="p-8">
        <h2 className="text-xl font-semibold text-red-600">测试运行未找到</h2>
        <button
          onClick={() => navigate("/test/history")}
          className="mt-4 px-4 py-2 bg-blue-600 text-white rounded hover:bg-blue-700"
        >
          返回历史记录
        </button>
      </div>
    );
  }

  const statusConfig = {
    queued: { label: "已排队", color: "bg-gray-200 text-gray-800" },
    preparing: { label: "准备中", color: "bg-blue-100 text-blue-800" },
    running: { label: "运行中", color: "bg-green-100 text-green-800" },
    completed: { label: "已完成", color: "bg-blue-100 text-blue-800" },
    failed: { label: "失败", color: "bg-red-100 text-red-800" },
    cancelled: { label: "已取消", color: "bg-yellow-100 text-yellow-800" },
  };

  const statusInfo = statusConfig[run.status] || statusConfig.queued;

  return (
    <div className="p-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold">测试运行详情</h1>
          <p className="text-gray-500 text-sm mt-1">ID: {runId}</p>
        </div>
        <div className="flex items-center gap-3">
          <span className={`px-3 py-1 rounded-full text-sm font-medium ${statusInfo.color}`}>
            {statusInfo.label}
          </span>
          {run.status === "running" && (
            <button
              onClick={handleStop}
              disabled={stopping}
              className="px-4 py-2 bg-red-600 text-white rounded hover:bg-red-700 disabled:opacity-50"
            >
              {stopping ? "停止中..." : "停止测试"}
            </button>
          )}
        </div>
      </div>

      {/* Main Grid */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Left Column: Info and Config */}
        <div className="lg:col-span-1 space-y-6">
          {/* Basic Info */}
          <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-4">
            <h2 className="text-lg font-semibold mb-4">基本信息</h2>
            <div className="space-y-2 text-sm">
              <div className="flex justify-between">
                <span className="text-gray-500">场景 ID</span>
                <span className="font-mono">{run.scenario_id}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-500">运行模式</span>
                <span>{run.run_mode}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-500">开始时间</span>
                <span>{new Date(run.started_at).toLocaleString()}</span>
              </div>
              {run.finished_at && (
                <div className="flex justify-between">
                  <span className="text-gray-500">结束时间</span>
                  <span>{new Date(run.finished_at).toLocaleString()}</span>
                </div>
              )}
              <div className="flex justify-between">
                <span className="text-gray-500">运行时长</span>
                <span>
                  {((run.finished_at || Date.now()) - run.started_at).toLocaleString()} ms
                </span>
              </div>
            </div>
          </div>

          {/* Test Config */}
          <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-4">
            <h2 className="text-lg font-semibold mb-4">测试配置</h2>
            <div className="space-y-2 text-sm">
              {run.config_snapshot.capture_type && (
                <div className="flex justify-between">
                  <span className="text-gray-500">捕获类型</span>
                  <span>{run.config_snapshot.capture_type}</span>
                </div>
              )}
              {run.config_snapshot.encoder_type && (
                <div className="flex justify-between">
                  <span className="text-gray-500">编码器</span>
                  <span>{run.config_snapshot.encoder_type}</span>
                </div>
              )}
              {run.config_snapshot.decoder_type && (
                <div className="flex justify-between">
                  <span className="text-gray-500">解码器</span>
                  <span>{run.config_snapshot.decoder_type}</span>
                </div>
              )}
              {run.config_snapshot.resolution && (
                <div className="flex justify-between">
                  <span className="text-gray-500">分辨率</span>
                  <span>
                    {run.config_snapshot.resolution[0]} x {run.config_snapshot.resolution[1]}
                  </span>
                </div>
              )}
              {run.config_snapshot.fps && (
                <div className="flex justify-between">
                  <span className="text-gray-500">帧率</span>
                  <span>{run.config_snapshot.fps} FPS</span>
                </div>
              )}
              {run.config_snapshot.bitrate && (
                <div className="flex justify-between">
                  <span className="text-gray-500">码率</span>
                  <span>{(run.config_snapshot.bitrate / 1000).toFixed(0)} kbps</span>
                </div>
              )}
            </div>
          </div>

          {/* Environment */}
          <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-4">
            <h2 className="text-lg font-semibold mb-4">测试环境</h2>
            <div className="space-y-2 text-sm">
              <div className="flex justify-between">
                <span className="text-gray-500">CPU</span>
                <span className="text-xs">{run.environment_snapshot.cpu_brand}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-500">核心数</span>
                <span>{run.environment_snapshot.cpu_cores}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-500">内存</span>
                <span>{run.environment_snapshot.memory_gb} GB</span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-500">GPU</span>
                <span className="text-xs">{run.environment_snapshot.gpu_info}</span>
              </div>
            </div>
          </div>
        </div>

        {/* Right Column: Events, Metrics, Summary */}
        <div className="lg:col-span-2 space-y-6">
          {/* Stage Events */}
          <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-4">
            <h2 className="text-lg font-semibold mb-4">阶段事件</h2>
            <div className="space-y-2 max-h-64 overflow-y-auto">
              {events.length === 0 ? (
                <p className="text-gray-500 text-sm">暂无事件</p>
              ) : (
                events.map((event, idx) => (
                  <div
                    key={idx}
                    className="flex items-center justify-between p-2 bg-gray-50 dark:bg-gray-700 rounded text-sm"
                  >
                    <div className="flex items-center gap-3">
                      <span className="font-medium">{event.stage}</span>
                      <span
                        className={`px-2 py-0.5 rounded text-xs ${
                          event.status === "started"
                            ? "bg-blue-100 text-blue-800"
                            : event.status === "completed"
                            ? "bg-green-100 text-green-800"
                            : event.status === "failed"
                            ? "bg-red-100 text-red-800"
                            : "bg-gray-100 text-gray-800"
                        }`}
                      >
                        {event.status}
                      </span>
                      {event.error && (
                        <span className="text-red-600 text-xs">{event.error}</span>
                      )}
                    </div>
                    <span className="text-gray-500 text-xs">
                      {new Date(event.timestamp).toLocaleTimeString()}
                    </span>
                  </div>
                ))
              )}
            </div>
          </div>

          {/* Summary */}
          {run.summary && (
            <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-4">
              <h2 className="text-lg font-semibold mb-4">测试结果</h2>
              <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
                <MetricCard
                  label="总时长"
                  value={`${run.summary.total_duration_ms.toLocaleString()} ms`}
                />
                <MetricCard
                  label="帧数"
                  value={run.summary.frame_count.toLocaleString()}
                />
                <MetricCard
                  label="丢帧"
                  value={run.summary.dropped_frames.toLocaleString()}
                  highlight={run.summary.dropped_frames > 0}
                />
                {run.summary.capture_fps && (
                  <MetricCard
                    label="采集 FPS"
                    value={run.summary.capture_fps.toFixed(1)}
                  />
                )}
                {run.summary.encode_latency_p50 && (
                  <MetricCard
                    label="编码延迟 P50"
                    value={`${run.summary.encode_latency_p50.toFixed(2)} ms`}
                  />
                )}
                {run.summary.encode_latency_p95 && (
                  <MetricCard
                    label="编码延迟 P95"
                    value={`${run.summary.encode_latency_p95.toFixed(2)} ms`}
                  />
                )}
                {run.summary.decode_latency_p50 && (
                  <MetricCard
                    label="解码延迟 P50"
                    value={`${run.summary.decode_latency_p50.toFixed(2)} ms`}
                  />
                )}
                {run.summary.decode_latency_p95 && (
                  <MetricCard
                    label="解码延迟 P95"
                    value={`${run.summary.decode_latency_p95.toFixed(2)} ms`}
                  />
                )}
                {run.summary.total_latency_p95 && (
                  <MetricCard
                    label="总延迟 P95"
                    value={`${run.summary.total_latency_p95.toFixed(2)} ms`}
                  />
                )}
              </div>

              {/* Error Message */}
              {run.summary.error_message && (
                <div className="mt-4 p-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded">
                  <p className="text-sm font-medium text-red-800 dark:text-red-200">错误</p>
                  <p className="text-sm text-red-600 dark:text-red-400 mt-1">
                    {run.summary.error_message}
                  </p>
                </div>
              )}

              {/* Failure Reason */}
              {run.summary.failure_reason && (
                <div className="mt-4 p-3 bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-800 rounded">
                  <p className="text-sm font-medium text-yellow-800 dark:text-yellow-200">失败原因</p>
                  <p className="text-sm text-yellow-600 dark:text-yellow-400 mt-1">
                    {run.summary.failure_reason}
                  </p>
                </div>
              )}
            </div>
          )}

          {/* Metrics Chart */}
          {run && (
            <TestTelemetryPanel runId={run.run_id} mode="fullPage" />
          )}
        </div>
      </div>
    </div>
  );
}

function MetricCard({
  label,
  value,
  highlight,
}: {
  label: string;
  value: string | number;
  highlight?: boolean;
}) {
  return (
    <div
      className={`p-3 rounded ${
        highlight
          ? "bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800"
          : "bg-gray-50 dark:bg-gray-700"
      }`}
    >
      <p className="text-xs text-gray-500 dark:text-gray-400">{label}</p>
      <p className="text-lg font-semibold mt-1">{value}</p>
    </div>
  );
}
