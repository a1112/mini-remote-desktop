import { createBrowserRouter } from "react-router";
import { Layout } from "./components/Layout";
import { HomePage } from "./components/HomePage";
import { DevicesPage } from "./components/DevicesPage";
import { DeviceDetailPage } from "./components/DeviceDetailPage";
import { RemoteSessionPage } from "./components/RemoteSessionPage";
import { TestPage } from "./components/TestPage";
// Test Workbench
import {
  WorkbenchLayout,
  OverviewPage,
  E2ETestPage,
  RunDetailPage,
  CaptureTestPage,
  EncodeTestPage,
  DecodeTestPage,
  RenderTestPage,
  TransportTestPage,
  CustomTestPage,
  MatrixTestPage,
  TestHistoryPage,
} from "./components/TestWorkbench";

export const router = createBrowserRouter([
  {
    path: "/session/:id",
    Component: RemoteSessionPage,
  },
  {
    path: "/",
    Component: Layout,
    children: [
      { index: true, Component: HomePage },
      { path: "devices", Component: DevicesPage },
      { path: "devices/:id", Component: DeviceDetailPage },
      // Legacy test page (kept for backward compatibility)
      { path: "test-legacy", Component: TestPage },
      { path: "*", Component: HomePage },
    ],
  },
  // Test Workbench - New unified test UI
  {
    path: "/test",
    Component: WorkbenchLayout,
    children: [
      { index: true, Component: OverviewPage },
      { path: "capture", Component: CaptureTestPage },
      { path: "encode", Component: EncodeTestPage },
      { path: "decode", Component: DecodeTestPage },
      { path: "render", Component: RenderTestPage },
      { path: "transport", Component: TransportTestPage },
      { path: "e2e", Component: E2ETestPage },
      { path: "custom", Component: CustomTestPage },
      { path: "matrix", Component: MatrixTestPage },
      { path: "history", Component: TestHistoryPage },
      { path: "run/:runId", Component: RunDetailPage },
    ],
  },
]);