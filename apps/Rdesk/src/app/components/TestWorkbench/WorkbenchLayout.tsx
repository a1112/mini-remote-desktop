import { Outlet, Link, useLocation, useNavigate } from "react-router";
import { useEffect, useState } from "react";
import type { CSSProperties, MouseEvent, ReactNode } from "react";
import {
  LayoutDashboard,
  Activity,
  Settings,
  Film,
  Gauge,
  Eye,
  ArrowRightLeft,
  Layers,
  History,
  Package,
  ArrowLeft,
  Home,
  LineChart,
  Minus,
  Square,
  X,
  Server,
  Cpu,
  MemoryStick,
  Monitor,
  Wifi,
} from "lucide-react";
import * as commands from "../../adapters/tauri/commands";
import type { SystemResourceSnapshot } from "../../adapters/tauri/types";
import { useShowUnavailableCapabilities } from "./useCapabilityVisibility";

const navigation = [
  { name: "Overview", href: "/test", icon: LayoutDashboard },
  { name: "Capture", href: "/test/capture", icon: Eye },
  { name: "Encode", href: "/test/encode", icon: Film },
  { name: "Decode", href: "/test/decode", icon: Gauge },
  { name: "Render", href: "/test/render", icon: Layers },
  { name: "Transport", href: "/test/transport", icon: ArrowRightLeft },
  { name: "E2E", href: "/test/e2e", icon: Activity },
  { name: "Custom", href: "/test/custom", icon: Settings },
  { name: "Matrix", href: "/test/matrix", icon: Package },
  { name: "History", href: "/test/history", icon: History },
  { name: "Telemetry", href: "/test/telemetry", icon: LineChart },
];

export function WorkbenchLayout() {
  const location = useLocation();
  const navigate = useNavigate();
  const [resourceSnapshot, setResourceSnapshot] =
    useState<SystemResourceSnapshot | null>(null);
  const [showUnavailable, setShowUnavailable] = useShowUnavailableCapabilities();

  const noDragSelector =
    'button, a, input, select, textarea, [role="button"], [data-no-drag="true"]';

  useEffect(() => {
    let cancelled = false;
    let inFlight = false;

    const refreshResources = async () => {
      if (inFlight) return;
      inFlight = true;

      try {
        const result = await commands.getSystemResourceSnapshot();
        if (!cancelled && result.ok) {
          setResourceSnapshot(result.value);
        }
      } finally {
        inFlight = false;
      }
    };

    void refreshResources();
    const intervalId = window.setInterval(() => void refreshResources(), 2000);

    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, []);

  const handleDragStart = (event: MouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest(noDragSelector)) return;
    event.preventDefault();
    void commands.startDragWindow();
  };

  const iconButton =
    "flex h-9 w-9 items-center justify-center text-muted-foreground transition-colors hover:bg-muted hover:text-foreground";

  return (
    <div className="workbench-theme flex h-screen flex-col overflow-hidden bg-background text-foreground">
      <header
        className="flex h-11 shrink-0 select-none items-center border-b bg-card/95"
        style={{ WebkitAppRegion: "drag" } as CSSProperties}
        onMouseDown={handleDragStart}
      >
        <div className="flex items-center gap-1 px-2">
          <button
            type="button"
            className={iconButton}
            style={{ WebkitAppRegion: "no-drag" } as CSSProperties}
            onClick={() => navigate(-1)}
            title="Back"
          >
            <ArrowLeft className="h-4 w-4" />
          </button>
          <button
            type="button"
            className={iconButton}
            style={{ WebkitAppRegion: "no-drag" } as CSSProperties}
            onClick={() => navigate("/")}
            title="Home"
          >
            <Home className="h-4 w-4" />
          </button>
        </div>

        <div className="flex min-w-0 flex-1 items-center gap-3 px-2">
          <div className="truncate text-sm font-semibold">Rdesk Test Workbench</div>
          <ResourceMonitorStrip snapshot={resourceSnapshot} />
        </div>

        <div
          className="flex h-full items-center"
          style={{ WebkitAppRegion: "no-drag" } as CSSProperties}
        >
          <button
            type="button"
            className={iconButton}
            onClick={() => void commands.minimizeWindow()}
            title="Minimize"
          >
            <Minus className="h-4 w-4" />
          </button>
          <button
            type="button"
            className={iconButton}
            onClick={() => void commands.toggleMaximizeWindow()}
            title="Maximize"
          >
            <Square className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            className="flex h-9 w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-destructive hover:text-destructive-foreground dark:hover:bg-red-500 dark:hover:text-white"
            onClick={() => void commands.closeWindow()}
            title="Close"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1">
        <aside className="flex min-h-0 w-64 shrink-0 flex-col border-r bg-card p-4">
          <div className="mb-6 shrink-0">
            <h1 className="text-xl font-bold text-foreground">Test Workbench</h1>
            <p className="text-sm text-muted-foreground">Rdesk media pipeline</p>
          </div>

          <nav className="min-h-0 flex-1 space-y-1 overflow-y-auto pr-1">
            {navigation.map((item) => {
              const isActive =
                location.pathname === item.href ||
                (item.href !== "/test" && location.pathname.startsWith(`${item.href}/`));
              return (
                <Link
                  key={item.name}
                  to={item.href}
                  className={`flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors ${
                    isActive
                      ? "bg-primary text-primary-foreground dark:bg-blue-600 dark:text-white"
                      : "text-muted-foreground hover:bg-muted hover:text-foreground"
                  }`}
                >
                  <item.icon className="h-4 w-4" />
                  {item.name}
                </Link>
              );
            })}
          </nav>

          <div className="shrink-0 space-y-3 border-t pt-4 text-xs text-muted-foreground">
            <label className="flex cursor-pointer items-center gap-2 rounded-md border bg-background/50 px-3 py-2 text-foreground transition-colors hover:bg-muted">
              <input
                type="checkbox"
                checked={showUnavailable}
                onChange={(event) => setShowUnavailable(event.target.checked)}
                className="h-4 w-4 accent-primary"
              />
              <span>显示不可用能力</span>
            </label>
            <div>WebRTC capture display path</div>
          </div>
        </aside>

        <main className="min-w-0 flex-1 overflow-auto bg-background">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

function ResourceMonitorStrip({
  snapshot,
}: {
  snapshot: SystemResourceSnapshot | null;
}) {
  const targetTitle = snapshot
    ? `${snapshot.target_name}${
        snapshot.target_pid != null ? ` PID ${snapshot.target_pid}` : ""
      }${snapshot.target_found ? "" : " not running"}`
    : "resource target";
  const targetValue = snapshot ? compactTargetName(snapshot.target_name) : "--";
  const gpuValue = formatGpuValue(snapshot);
  const memoryTitle = snapshot
    ? `${targetTitle} memory ${formatMemory(snapshot.memory_used_mb)} (${formatPercent(
        snapshot.memory_usage_percent
      )} of system memory)`
    : "target memory";
  const gpuScope = snapshot?.gpu_metrics_scope ?? "unavailable";
  const gpuTitle = snapshot?.gpu_metrics_available
    ? `${formatScopeLabel(gpuScope, targetTitle)} GPU ${gpuValue}`
    : "GPU metrics unavailable";
  const networkScope = snapshot?.network_metrics_scope ?? "unavailable";
  const networkTitle = snapshot
    ? snapshot.network_metrics_available
      ? `${formatScopeLabel(networkScope, targetTitle)} network Rx ${formatRate(snapshot.network_rx_bps)} Tx ${formatRate(
          snapshot.network_tx_bps
        )}`
      : "network metrics unavailable"
    : "network";

  return (
    <div className="hidden min-w-0 items-center gap-1 text-muted-foreground lg:flex">
      <TitleMetric
        icon={<Server className="h-3.5 w-3.5" />}
        label="SRC"
        value={targetValue}
        title={targetTitle}
        wide
      />
      <TitleMetric
        icon={<Cpu className="h-3.5 w-3.5" />}
        label="CPU"
        value={snapshot ? formatPercent(snapshot.cpu_usage_percent) : "--"}
        title={`${targetTitle} CPU usage`}
      />
      <TitleMetric
        icon={<MemoryStick className="h-3.5 w-3.5" />}
        label="MEM"
        value={snapshot ? formatMemory(snapshot.memory_used_mb) : "--"}
        title={memoryTitle}
      />
      <TitleMetric
        icon={<Monitor className="h-3.5 w-3.5" />}
        label="GPU"
        value={gpuValue}
        title={gpuTitle}
      />
      <TitleMetric
        icon={<Wifi className="h-3.5 w-3.5" />}
        label="NET"
        value={
          snapshot?.network_metrics_available
            ? `R ${formatRate(snapshot.network_rx_bps)} T ${formatRate(
                snapshot.network_tx_bps
              )}`
            : "--"
        }
        title={networkTitle}
        wide
      />
    </div>
  );
}

function TitleMetric({
  icon,
  label,
  value,
  title,
  wide = false,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  title: string;
  wide?: boolean;
}) {
  return (
    <div
      className={`flex h-7 min-w-0 items-center gap-1.5 border-l border-border/70 pl-2 ${
        wide ? "max-w-[180px]" : "max-w-[90px]"
      }`}
      title={title}
    >
      <span className="shrink-0 text-muted-foreground">{icon}</span>
      <span className="shrink-0 text-[10px] font-semibold text-muted-foreground">
        {label}
      </span>
      <span className="truncate font-mono text-[11px] font-semibold text-foreground">
        {value}
      </span>
    </div>
  );
}

function formatPercent(value: number) {
  if (!Number.isFinite(value)) return "--";
  return `${value.toFixed(value < 10 ? 1 : 0)}%`;
}

function formatMemory(valueMb: number) {
  if (!Number.isFinite(valueMb)) return "--";
  if (valueMb >= 1024) {
    return `${(valueMb / 1024).toFixed(1)} GB`;
  }
  return `${Math.round(valueMb)} MB`;
}

function formatRate(bytesPerSecond: number) {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) {
    return "0B/s";
  }

  const units = ["B/s", "K/s", "M/s", "G/s"];
  let value = bytesPerSecond;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)}${units[unitIndex]}`;
}

function formatGpuValue(snapshot: SystemResourceSnapshot | null) {
  if (!snapshot?.gpu_metrics_available) return "--";
  if (snapshot.gpu_usage_percent != null) {
    return formatPercent(snapshot.gpu_usage_percent);
  }
  if (snapshot.gpu_memory_used_mb != null) {
    return formatMemory(snapshot.gpu_memory_used_mb);
  }
  return "--";
}

function compactTargetName(targetName: string) {
  if (targetName === "Rdesk Workbench") return "Workbench";
  return targetName;
}

function formatScopeLabel(scope: string | undefined, targetTitle: string) {
  switch (scope) {
    case "process":
      return targetTitle;
    case "system":
      return "System";
    default:
      return "Unavailable";
  }
}
