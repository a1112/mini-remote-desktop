import { Outlet, useNavigate } from "react-router";
import { Sidebar } from "./Sidebar";
import { TitleBar } from "./TitleBar";
import { ConnectionsModal } from "./ConnectionsModal";
import { SettingsModal } from "./SettingsModal";
import { TransferModal } from "./FileTransferPage";
import { AuthModal } from "./AuthModal";
import { useState } from "react";
import { useTheme } from "./ThemeContext";
import { DetailBarProvider } from "./DetailBarContext";
import { ServiceStatusPanel } from "./ServiceStatusPanel";
import { MobileLayout } from "./MobileLayout";
import { useIsMobile } from "./ui/use-mobile";
import { isTauriRuntime } from "../utils/runtime";

export function Layout() {
  const [collapsed, setCollapsed] = useState(false);
  const [showConnections, setShowConnections] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showTransfers, setShowTransfers] = useState(false);
  const [showAuth, setShowAuth] = useState(false);
  const { isDark } = useTheme();
  const narrow = useIsMobile();
  const native = isTauriRuntime();
  const isMobile = narrow && !native;
  const navigate = useNavigate();

  return (
    <DetailBarProvider>
      {isMobile ? <MobileLayout onOpenAuth={() => setShowAuth(true)} /> : <div
        className={`remote-shell flex flex-col overflow-hidden border ${isDark ? "dark border-gray-700" : "border-gray-300"}`}
      >
        <div className="flex flex-1 overflow-hidden">
          <Sidebar collapsed={collapsed} />
          <div className="flex min-w-0 flex-col flex-1 overflow-hidden">
            <TitleBar
              onOpenConnections={() => setShowConnections(true)}
              onOpenSettings={() => setShowSettings(true)}
              onOpenTransfers={() => setShowTransfers(true)}
              onOpenAuth={() => setShowAuth(true)}
              onSearchDevices={(query) => navigate(query ? `/devices?${new URLSearchParams({ q: query })}` : "/devices")}
              collapsed={collapsed}
              onToggleSidebar={() => setCollapsed(!collapsed)}
            />
            {native && <ServiceStatusPanel />}
            <main aria-label="Rdesk 工作区" className="flex-1 overflow-y-auto overflow-x-hidden">
              <Outlet />
            </main>
          </div>
        </div>
      </div>}
      <ConnectionsModal open={showConnections} onClose={() => setShowConnections(false)} />
      <SettingsModal open={showSettings} onClose={() => setShowSettings(false)} />
      <TransferModal open={showTransfers} onClose={() => setShowTransfers(false)} />
      <AuthModal open={showAuth} onClose={() => setShowAuth(false)} />
    </DetailBarProvider>
  );
}
