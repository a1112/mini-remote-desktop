import { createElement } from "react";
import { createBrowserRouter } from "react-router";
import { Layout } from "./components/Layout";
import { AppErrorScreen } from "./components/AppErrorScreen";
import { HomePage } from "./components/HomePage";
import { ConnectionsPage } from "./components/ConnectionsPage";
import { DevicesPage } from "./components/DevicesPage";
import { DeviceDetailPage } from "./components/DeviceDetailPage";
import { RemoteSessionPage } from "./components/RemoteSessionPage";
import { BrowserRemoteSessionPage } from "./components/BrowserRemoteSessionPage";

const loadTelemetryPage = async () => (await import("./components/TestWorkbench/TestTelemetryPage")).TestTelemetryPage;

function RouteLoading() {
  return createElement("p", { role: "status", className: "grid min-h-dvh place-items-center text-sm" }, "正在加载页面…");
}

export const router = createBrowserRouter([
  {
    path: "/browser-session/:id",
    Component: BrowserRemoteSessionPage,
    ErrorBoundary: AppErrorScreen,
  },
  {
    path: "/session/:id",
    Component: RemoteSessionPage,
    ErrorBoundary: AppErrorScreen,
  },
  {
    path: "/display/:id",
    lazy: { Component: async () => (await import("./components/RemoteDisplayWindowPage")).RemoteDisplayWindowPage },
    ErrorBoundary: AppErrorScreen,
    HydrateFallback: RouteLoading,
  },
  {
    path: "/",
    Component: Layout,
    ErrorBoundary: AppErrorScreen,
    HydrateFallback: RouteLoading,
    children: [
      { index: true, Component: HomePage },
      { path: "connections", Component: ConnectionsPage },
      { path: "devices", Component: DevicesPage },
      { path: "devices/:id", Component: DeviceDetailPage },
      // Legacy test page (kept for backward compatibility)
      { path: "test-legacy", lazy: { Component: async () => (await import("./components/TestPage")).TestPage } },
      { path: "*", Component: HomePage },
    ],
  },
  // Test Workbench - New unified test UI
  {
    path: "/test",
    lazy: { Component: async () => (await import("./components/TestWorkbench/WorkbenchLayout")).WorkbenchLayout },
    ErrorBoundary: AppErrorScreen,
    HydrateFallback: RouteLoading,
    children: [
      { index: true, lazy: { Component: async () => (await import("./components/TestWorkbench/OverviewPage")).OverviewPage } },
      { path: "capture", lazy: { Component: async () => (await import("./components/TestWorkbench/CaptureTestPage")).CaptureTestPage } },
      { path: "encode", lazy: { Component: async () => (await import("./components/TestWorkbench/EncodeTestPage")).EncodeTestPage } },
      { path: "decode", lazy: { Component: async () => (await import("./components/TestWorkbench/DecodeTestPage")).DecodeTestPage } },
      { path: "render", lazy: { Component: async () => (await import("./components/TestWorkbench/RenderTestPage")).RenderTestPage } },
      { path: "transport", lazy: { Component: async () => (await import("./components/TestWorkbench/TransportTestPage")).TransportTestPage } },
      { path: "e2e", lazy: { Component: async () => (await import("./components/TestWorkbench/E2ETestPage")).E2ETestPage } },
      { path: "custom", lazy: { Component: async () => (await import("./components/TestWorkbench/CustomTestPage")).CustomTestPage } },
      { path: "matrix", lazy: { Component: async () => (await import("./components/TestWorkbench/MatrixTestPage")).MatrixTestPage } },
      { path: "performance", lazy: { Component: async () => (await import("./components/TestWorkbench/PerformanceComparisonPage")).PerformanceComparisonPage } },
      { path: "history", lazy: { Component: async () => (await import("./components/TestWorkbench/TestHistoryPage")).TestHistoryPage } },
      { path: "telemetry", lazy: { Component: loadTelemetryPage } },
      { path: "telemetry/:runId", lazy: { Component: loadTelemetryPage } },
      { path: "run/:runId", lazy: { Component: async () => (await import("./components/TestWorkbench/RunDetailPage")).RunDetailPage } },
    ],
  },
], { basename: import.meta.env.BASE_URL });
