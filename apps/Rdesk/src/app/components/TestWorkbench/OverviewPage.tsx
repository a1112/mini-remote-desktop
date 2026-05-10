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
import {
  buildCapabilitySnapshotFromIpc,
  buildCapabilitySnapshotFromEnvironment,
  evaluateProfileSupport,
  type CapabilityDomain,
  type CapabilityItem,
  type CapabilitySnapshot,
  type CapabilityStatus,
} from "../../services/capabilityMatrix";

const CAPABILITY_DOMAIN_ORDER: CapabilityDomain[] = [
  "capture",
  "capture_source",
  "encode",
  "decode",
  "render",
  "memory",
  "transport",
  "control",
  "audio",
  "service",
  "security",
];

export function OverviewPage() {
  const navigate = useNavigate();
  const [scenarios, setScenarios] = useState<TestScenario[]>([]);
  const [recentRuns, setRecentRuns] = useState<TestRun[]>([]);
  const [capabilities, setCapabilities] = useState<EnvironmentSnapshot | null>(null);
  const [serviceCapabilitySnapshot, setServiceCapabilitySnapshot] =
    useState<CapabilitySnapshot | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    loadOverviewData();
  }, []);

  async function loadOverviewData() {
    setLoading(true);
    try {
      const [scenariosResult, runsResult, capsResult, serviceCapsResult] = await Promise.all([
        commands.testListScenarios(),
        commands.testListRuns({ limit: 5 }),
        commands.testGetCapabilities(),
        commands.ipcCapabilitySnapshot(),
      ]);

      if (scenariosResult.ok) setScenarios(scenariosResult.value);
      if (runsResult.ok) setRecentRuns(runsResult.value);
      if (capsResult.ok) setCapabilities(capsResult.value);
      if (serviceCapsResult.ok) {
        setServiceCapabilitySnapshot(buildCapabilitySnapshotFromIpc(serviceCapsResult.value));
      }
    } catch (error) {
      console.error("Failed to load overview data:", error);
    } finally {
      setLoading(false);
    }
  }

  const successfulRuns = recentRuns.filter((r) => r.status === "completed").length;
  const failedRuns = recentRuns.filter((r) => r.status === "failed").length;
  const capabilitySnapshot =
    serviceCapabilitySnapshot ??
    (capabilities ? buildCapabilitySnapshotFromEnvironment(capabilities) : null);
  const capabilityGroups = capabilitySnapshot
    ? groupCapabilitiesByDomain(capabilitySnapshot.capabilities)
    : [];
  const lan2k144Evaluation = capabilitySnapshot
    ? evaluateProfileSupport("lan.2k144", capabilitySnapshot)
    : null;

  return (
    <div className="p-6">
      {/* Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-foreground">测试工作台总览</h1>
        <p className="text-muted-foreground">查看 CapTest 同口径链路、环境能力和最近运行结果</p>
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

          {/* Structured Capability Matrix */}
          {capabilitySnapshot && (
            <section className="bg-card rounded-lg border p-6">
              <div className="mb-4 flex flex-col gap-2 md:flex-row md:items-start md:justify-between">
                <div>
                  <h2 className="text-lg font-semibold">结构化能力矩阵</h2>
                  <p className="text-sm text-muted-foreground">
                    按 domain 展示当前机器能力、降级路径和不可用原因。
                  </p>
                </div>
                <div className="rounded-lg border bg-background/70 px-3 py-2 text-sm">
                  <div className="text-xs text-muted-foreground">Profile readiness</div>
                  <div className="mt-1 flex flex-wrap items-center gap-2">
                    <span className="font-medium">lan.2k144</span>
                    <StatusBadge status={lan2k144Evaluation?.status ?? "blocked"} />
                  </div>
                </div>
              </div>

              {lan2k144Evaluation && lan2k144Evaluation.reasons.length > 0 && (
                <div className="mb-4 rounded-lg border border-yellow-500/30 bg-yellow-500/10 p-3 text-xs text-yellow-700 dark:text-yellow-200">
                  {lan2k144Evaluation.reasons.join("; ")}
                </div>
              )}

              <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                {capabilityGroups.map(({ domain, items }) => (
                  <div key={domain} className="rounded-lg border bg-background/60 p-3">
                    <div className="mb-2 flex items-center justify-between">
                      <h3 className="text-sm font-semibold">{domain}</h3>
                      <span className="text-xs text-muted-foreground">{items.length}</span>
                    </div>
                    <div className="space-y-2">
                      {items.slice(0, 4).map((item) => (
                        <div key={item.id} className="rounded border bg-card/60 px-2 py-1.5">
                          <div className="flex items-center justify-between gap-2">
                            <span className="truncate text-xs font-medium">{item.id}</span>
                            <StatusBadge status={item.status} />
                          </div>
                          {item.reason && (
                            <div className="mt-1 line-clamp-2 text-[11px] text-muted-foreground">
                              {item.reason}
                            </div>
                          )}
                        </div>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            </section>
          )}

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
                  <div className="text-sm text-muted-foreground">采集到渲染的直接性能基线</div>
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
                  <div className="text-sm text-muted-foreground">DXGI/WGC、NVENC、NVDEC、QUIC 同口径组合</div>
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
                  <div className="text-sm text-muted-foreground">H.264/HEVC/Main10/AV1 单链路调试</div>
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

function groupCapabilitiesByDomain(capabilities: CapabilityItem[]): Array<{
  domain: CapabilityDomain;
  items: CapabilityItem[];
}> {
  return CAPABILITY_DOMAIN_ORDER.map((domain) => ({
    domain,
    items: capabilities.filter((capability) => capability.domain === domain),
  })).filter((group) => group.items.length > 0);
}

function StatusBadge({ status }: { status: CapabilityStatus | "ready" | "blocked" | "skipped" }) {
  return (
    <span
      className={[
        "rounded-full px-2 py-0.5 text-[10px] font-semibold",
        statusClassName(status),
      ].join(" ")}
    >
      {status}
    </span>
  );
}

function statusClassName(status: CapabilityStatus | "ready" | "blocked" | "skipped"): string {
  switch (status) {
    case "available":
    case "usable":
    case "ready":
      return "bg-green-500/12 text-green-600 dark:text-green-300";
    case "degraded":
      return "bg-yellow-500/12 text-yellow-700 dark:text-yellow-300";
    case "blocked":
    case "permission_missing":
    case "driver_missing":
    case "hardware_missing":
      return "bg-red-500/12 text-red-600 dark:text-red-300";
    case "unimplemented":
    case "unsupported":
    case "skipped":
      return "bg-slate-500/12 text-slate-600 dark:text-slate-300";
    default:
      return "bg-muted text-muted-foreground";
  }
}
