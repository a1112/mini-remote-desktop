import { ArrowLeft, Keyboard, Monitor, MoreHorizontal, Mouse, X, ZoomIn } from "lucide-react";

type MobileSessionViewProps = {
  deviceName: string;
  status: string;
  awaitingApproval: boolean;
  streaming: boolean;
  ended?: boolean;
  hasNativeDisplay: boolean;
  canOpenDisplay: boolean;
  firstFrameStatus?: string | null;
  error: string | null;
  suggestedAction: string | null;
  onDisconnect: () => void;
  onOpenDisplay: () => void;
};

export function MobileSessionView({
  deviceName, status, awaitingApproval, streaming, ended = false, hasNativeDisplay, canOpenDisplay, firstFrameStatus,
  error, suggestedAction, onDisconnect, onOpenDisplay,
}: MobileSessionViewProps) {
  const waiting = !streaming && !error;
  const canvasTitle = error || ended ? "远程画面不可用" : hasNativeDisplay ? "显示窗口已打开" : "等待远程画面";
  const canvasDescription = ended
    ? null
    : error
      ? "请检查连接状态后重新发起请求"
      : hasNativeDisplay
        ? firstFrameStatus ?? "请前往独立窗口查看"
        : awaitingApproval
          ? "请在远程设备上确认本次连接请求"
          : streaming
            ? "当前会话尚无可在此页面显示的画面"
            : status;

  return <div className="mobile-session-shell">
    <header className="mobile-session-header">
      <button type="button" aria-label={ended ? "返回" : "返回并断开"} onClick={onDisconnect}><ArrowLeft size={23} /></button>
      <h1>远程会话</h1>
      <span role="status" className={`mobile-session-status ${error ? "error" : streaming ? "connected" : ""}`}>{status}</span>
    </header>
    <div className="mobile-session-device">{deviceName}</div>
    <main className="mobile-session-canvas">
      <div className="mobile-session-center">
        <Monitor size={82} strokeWidth={1.4} aria-hidden="true" />
        <h2>{canvasTitle}</h2>
        {canvasDescription && <p>{canvasDescription}</p>}
        {error ? <p className="mobile-session-error" role="alert">{error}</p> : null}
        {suggestedAction ? <p className="mobile-session-suggestion">{suggestedAction}</p> : null}
        {ended ? <button type="button" className="mobile-session-open" onClick={onDisconnect}>返回设备列表</button>
          : canOpenDisplay ? <button type="button" className="mobile-session-open" onClick={onOpenDisplay}>打开远程画面</button> : null}
      </div>
    </main>
    {!ended && <div className="mobile-session-bottom">
      <p>{waiting ? "正在等待连接状态更新" : "触控控制尚未接入当前显示通道"}</p>
      <div className="mobile-session-toolbar" aria-label="会话操作">
        {[{ label: "鼠标", icon: Mouse }, { label: "键盘", icon: Keyboard },
          { label: "缩放", icon: ZoomIn }, { label: "更多", icon: MoreHorizontal }].map(({ label, icon: Icon }) =>
          <button key={label} disabled title="当前显示通道暂不支持触控操作"><span><Icon size={22} /></span>{label}</button>,
        )}
        <button className="mobile-session-end" onClick={onDisconnect}><span><X size={25} /></span>结束</button>
      </div>
    </div>}
  </div>;
}
