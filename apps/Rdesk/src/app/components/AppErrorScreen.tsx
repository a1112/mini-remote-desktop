import { Component, type PropsWithChildren } from "react";
import { CircleAlert } from "lucide-react";

export function AppErrorScreen() {
  const home = import.meta.env.BASE_URL;
  return (
    <main className="app-error-screen">
      <section className="remote-home-card">
        <span className="remote-home-icon"><CircleAlert size={22} aria-hidden="true" /></span>
        <h1>无法加载当前页面</h1>
        <p>重新加载页面，或返回远程控制首页。</p>
        <div>
          <button className="remote-home-primary" type="button" onClick={() => window.location.reload()}>重新加载</button>
          {window.location.pathname !== home && <a className="remote-home-link" href={home}>返回首页</a>}
        </div>
      </section>
    </main>
  );
}

export class AppErrorBoundary extends Component<PropsWithChildren, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  render() {
    return this.state.failed ? <AppErrorScreen /> : this.props.children;
  }
}
