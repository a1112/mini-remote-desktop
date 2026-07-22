import { useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router";
import { testListRuns } from "../../adapters/tauri/commands";
import type { TestRun } from "../../adapters/tauri/types";
import { TestTelemetryPanel } from "./TestTelemetryPanel";

export function TestTelemetryPage() {
  const { runId } = useParams();
  const navigate = useNavigate();
  const [runs, setRuns] = useState<TestRun[]>([]);
  const [selectedRunId, setSelectedRunId] = useState(runId ?? "");
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    async function loadRuns() {
      setLoading(true);
      const result = await testListRuns({ limit: 100 });
      if (!cancelled && result.ok) {
        setRuns(result.value);
        if (!runId && !selectedRunId && result.value[0]) {
          setSelectedRunId(result.value[0].run_id);
        }
      }
      if (!cancelled) setLoading(false);
    }
    void loadRuns();
    return () => {
      cancelled = true;
    };
  }, [runId, selectedRunId]);

  useEffect(() => {
    if (runId) setSelectedRunId(runId);
  }, [runId]);

  const selectedRun = useMemo(
    () => runs.find((run) => run.run_id === selectedRunId),
    [runs, selectedRunId]
  );

  const selectRun = (nextRunId: string) => {
    setSelectedRunId(nextRunId);
    if (nextRunId) navigate(`/test/telemetry/${nextRunId}`);
  };

  return (
    <div className="mx-auto max-w-7xl p-6">
      <div className="mb-6 flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold">测试曲线</h1>
          <p className="mt-1 text-sm text-gray-500">按时间查看测试指标、日志和 artifacts</p>
        </div>
        <div className="min-w-[18rem]">
          <label className="mb-1 block text-sm font-medium" htmlFor="telemetry-run-select">
            测试记录
          </label>
          <select
            id="telemetry-run-select"
            value={selectedRunId}
            onChange={(event) => selectRun(event.target.value)}
            className="w-full rounded border px-3 py-2 dark:border-gray-600 dark:bg-gray-700"
          >
            <option value="">选择测试记录</option>
            {runs.map((run) => (
              <option key={run.run_id} value={run.run_id}>
                {run.scenario_id} · {new Date(run.started_at).toLocaleString()}
              </option>
            ))}
          </select>
        </div>
      </div>

      {loading && !selectedRunId ? (
        <div className="flex h-64 items-center justify-center">
          <div className="h-6 w-6 animate-spin rounded-full border-2 border-muted border-t-primary" />
        </div>
      ) : selectedRunId ? (
        <TestTelemetryPanel
          runId={selectedRunId}
          mode="fullPage"
          className={selectedRun ? "" : "border-yellow-300"}
        />
      ) : (
        <div className="rounded-lg border border-dashed border-gray-300 p-8 text-center text-gray-500 dark:border-gray-700">
          暂无可查看的测试记录
        </div>
      )}
    </div>
  );
}
