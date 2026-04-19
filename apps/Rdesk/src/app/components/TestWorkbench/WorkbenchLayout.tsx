import { Outlet, Link, useLocation } from "react-router";
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
} from "lucide-react";

const navigation = [
  { name: "总览", href: "/test", icon: LayoutDashboard },
  { name: "采集测试", href: "/test/capture", icon: Eye },
  { name: "编码测试", href: "/test/encode", icon: Film },
  { name: "解码测试", href: "/test/decode", icon: Gauge },
  { name: "渲染测试", href: "/test/render", icon: Layers },
  { name: "传输测试", href: "/test/transport", icon: ArrowRightLeft },
  { name: "端到端测试", href: "/test/e2e", icon: Activity },
  { name: "自由组合", href: "/test/custom", icon: Settings },
  { name: "矩阵测试", href: "/test/matrix", icon: Package },
  { name: "历史记录", href: "/test/history", icon: History },
];

export function WorkbenchLayout() {
  const location = useLocation();

  return (
    <div className="flex h-screen bg-background">
      {/* Sidebar Navigation */}
      <aside className="w-64 border-r bg-card p-4">
        <div className="mb-6">
          <h1 className="text-xl font-bold text-foreground">测试工作台</h1>
          <p className="text-sm text-muted-foreground">Rdesk Test Workbench</p>
        </div>

        <nav className="space-y-1">
          {navigation.map((item) => {
            const isActive = location.pathname === item.href;
            return (
              <Link
                key={item.name}
                to={item.href}
                className={`flex items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors ${
                  isActive
                    ? "bg-primary text-primary-foreground"
                    : "text-muted-foreground hover:bg-muted hover:text-foreground"
                }`}
              >
                <item.icon className="h-4 w-4" />
                {item.name}
              </Link>
            );
          })}
        </nav>

        <div className="mt-auto pt-4 border-t text-xs text-muted-foreground">
          <p>环境能力检测中...</p>
        </div>
      </aside>

      {/* Main Content Area */}
      <main className="flex-1 overflow-auto">
        <Outlet />
      </main>
    </div>
  );
}
