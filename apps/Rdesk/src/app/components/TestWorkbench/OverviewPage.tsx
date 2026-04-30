import { useEffect, useState } from "react";
import { useNavigate } from "react-router";
import {
  Activity,
  CheckCircle2,
  XCircle,
  Clock,
  Zap,
  Monitor,
  ArrowRight,
} from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type { TestScenario, TestRun, EnvironmentSnapshot } from "../../adapters/tauri/types";

export function OverviewPage() {
  const navigate = useNavigate();
  const [scenarios, setScenarios] = useState<TestScenario[]>([]);
  const [recentRuns, setRecentRuns] = useState<TestRun[]>([]);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    loadOverviewData();
  }, []);

  async function loadOverviewData() {
    setLoading(true);
    try {
      const [scenariosResult, runsResult, capsResult] = await Promise.all([
        commands.testListScenarios(),
        commands.testListRuns({ limit: 5 }),
        commands.testGetCapabilities(),
      ]);

      if (scenariosResult.ok) setScenarios(scenariosResult.value);
      if (runsResult.ok) setRecentRuns(runsResult.value);
      if (capsResult.ok) setCapabilities(capsResult.value);
    } catch (error) {
      console.error("Failed to load overview data:", error);
    } finally {
      setLoading(false);
    }
  }

  const successfulRuns = recentRuns.filter((r) => r.status === "completed").length;
  const failedRuns = recentRuns.filter((r) => r.status === "failed").length;

  return (
    <div className="p-6">
      {/* Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-foreground">测试工作台总览</h1>
        <p className="text-muted-foreground">查看测试环境状态和最近运行结果</p>
      </div>

      {loading ? (
        <div className="flex items-center justify-center h-64">
          <div className="text-muted-foreground">加载中...</div>
        </div>
      ) : (
        <div className="space-y-6">
          {/* Environment Summary */}
          <section className="bg-card rounded-lg border p-6">
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <Monitor className="h-5 w-5" />
              环境摘要
            </h2>
            {capabilities && (
              <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
                <div>
                  <span className="text-muted-foreground">CPU:</span>{" "}
                  {capabilities.cpu_brand}
                </div>
                <div>
                  <span className="text-muted-foreground">核心数:</span>{" "}
                  {capabilities.cpu_cores}
                </div>
                <div>
                  <span className="text-muted-foreground">内存:</span>{" "}
                  {capabilities.memory_gb} GB
                </div>
                <div>
                  <span className="text-muted-foreground">GPU:</span>{" "}
                  {capabilities.gpu_info}
                </div>
              </div>
            )}
          </section>

          {/* Quick Stats */}
          <section className="grid grid-cols-1 md:grid-cols-4 gap-4">
            <div className="bg-card rounded-lg border p-4">
              <div className="flex items-center gap-2 text-muted-foreground text-sm mb-1">
                <Activity className="h-4 w-4" />
                <span>总测试场景</span>
              </div>
              <div className="text-2xl font-semibold">{scenarios.length}</div>
            </div>
            <div className="bg-card rounded-lg border p-4">
              <div className="flex items-center gap-2 text-muted-foreground text-sm mb-1">
                <Clock className="h-4 w-4" />
                <span>最近运行</span>
              </div>
              <div className="text-2xl font-semibold">{recentRuns.length}</div>
            </div>
            <div className="bg-card rounded-lg border p-4">
              <div className="flex items-center gap-2 text-green-500 text-sm mb-1">
                <CheckCircle2 className="h-4 w-4" />
                <span>成功</span>
              </div>
              <div className="text-2xl font-semibold">{successfulRuns}</div>
            </div>
            <div className="bg-card rounded-lg border p-4">
              <div className="flex items-center gap-2 text-red-500 text-sm mb-1">
                <XCircle className="h-4 w-4" />
                <span>失败</span>
              </div>
              <div className="text-2xl font-semibold">{failedRuns}</div>
            </div>
          </section>

          {/* Quick Actions */}
          <section className="bg-card rounded-lg border p-6">
            <h2 className="text-lg font-semibold mb-4 flex items-center gap-2">
              <Zap className="h-5 w-5" />
              快速入口
            </h2>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              <button
                type="button"
                onClick={() => navigate("/test/e2e")}
                className="flex items-center justify-between rounded-lg border p-4 hover:bg-muted transition-colors text-left"
              >
                <div>
                  <div className="font-medium">端到端本地测试</div>
                  <div className="text-sm text-muted-foreground">测试完整采集到渲染流程</div>
                </div>
                <ArrowRight className="h-5 w-5 text-muted-foreground" />
              </button>
              <button
                type="button"
                onClick={() => navigate("/test/matrix")}
                className="flex items-center justify-between rounded-lg border p-4 hover:bg-muted transition-colors text-left"
              >
                <div>
                  <div className="font-medium">矩阵性能测试</div>
                  <div className="text-sm text-muted-foreground">批量测试多组参数组合</div>
                </div>
                <ArrowRight className="h-5 w-5 text-muted-foreground" />
              </button>
              <button
                type="button"
                onClick={() => navigate("/test/custom")}
                className="flex items-center justify-between rounded-lg border p-4 hover:bg-muted transition-colors text-left"
              >
                <div>
                  <div className="font-medium">自由组合测试</div>
                  <div className="text-sm text-muted-foreground">自定义测试配置</div>
                </div>
                <ArrowRight className="h-5 w-5 text-muted-foreground" />
              </button>
            </div>
          </section>

          {/* Recent Runs */}
          {recentRuns.length > 0 && (
            <section className="bg-card rounded-lg border p-6">
              <h2 className="text-lg font-semibold mb-4">最近运行</h2>
              <div className="space-y-2">
                {recentRuns.map((run) => (
                  <div
                    key={run.run_id}
                    className="flex items-center justify-between rounded-lg border p-3 hover:bg-muted transition-colors"
                  >
                    <div className="flex items-center gap-3">
                      {run.status === "completed" ? (
                        <CheckCircle2 className="h-5 w-5 text-green-500" />
                      ) : run.status === "failed" ? (
                        <XCircle className="h-5 w-5 text-red-500" />
                      ) : (
                        <Activity className="h-5 w-5 text-yellow-500 animate-spin" />
                      )}
                      <div>
                        <div className="font-medium">{run.scenario_id}</div>
                        <div className="text-sm text-muted-foreground">
                          {new Date(run.started_at).toLocaleString()}
                        </div>
                      </div>
                    </div>
                    <div className="flex items-center gap-4">
                      {run.summary && (
                        <div className="text-sm text-muted-foreground">
                          {run.summary.frame_count} 帧 · {run.summary.dropped_frames} 丢帧
                        </div>
                      )}
                      <button
                        type="button"
                        onClick={() => navigate(`/test/run/${run.run_id}`)}
                        className="text-sm text-primary hover:underline"
                      >
                        查看详情
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            </section>
          )}
        </div>
      )}
    </div>
  );
}
