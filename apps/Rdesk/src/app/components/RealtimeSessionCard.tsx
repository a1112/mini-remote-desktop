import { useTheme } from "./ThemeContext";

interface RealtimeSessionCardProps {
  deviceId: string;
  sessionId: string;
  targetDeviceId: string;
  offerSdp: string;
  answerSdp: string;
  iceCandidate: string;
  iceSdpMid: string;
  iceSdpMlineIndex: number;
  snapshotLocalOffer: string;
  snapshotRemoteOffer: string;
  snapshotRemoteAnswer: string;
  snapshotRemoteIceCount: number;
  handle: number | null;
  loading: boolean;
  error: string | null;
  events: string[];
  onDeviceIdChange: (value: string) => void;
  onSessionIdChange: (value: string) => void;
  onTargetDeviceIdChange: (value: string) => void;
  onOfferSdpChange: (value: string) => void;
  onAnswerSdpChange: (value: string) => void;
  onIceCandidateChange: (value: string) => void;
  onIceSdpMidChange: (value: string) => void;
  onIceSdpMlineIndexChange: (value: number) => void;
  onRegister: () => void;
  onRequest: () => void;
  onAccept: () => void;
  onSendOffer: () => void;
  onSendAnswer: () => void;
  onSendIceCandidate: () => void;
  onRefreshEvents: () => void;
  onSyncSnapshot: () => void;
}

export function RealtimeSessionCard({
  deviceId,
  sessionId,
  targetDeviceId,
  offerSdp,
  answerSdp,
  iceCandidate,
  iceSdpMid,
  iceSdpMlineIndex,
  snapshotLocalOffer,
  snapshotRemoteOffer,
  snapshotRemoteAnswer,
  snapshotRemoteIceCount,
  handle,
  loading,
  error,
  events,
  onDeviceIdChange,
  onSessionIdChange,
  onTargetDeviceIdChange,
  onOfferSdpChange,
  onAnswerSdpChange,
  onIceCandidateChange,
  onIceSdpMidChange,
  onIceSdpMlineIndexChange,
  onRegister,
  onRequest,
  onAccept,
  onSendOffer,
  onSendAnswer,
  onSendIceCandidate,
  onRefreshEvents,
  onSyncSnapshot,
}: RealtimeSessionCardProps) {
  const { isDark } = useTheme();

  return (
    <div className={`p-3.5 rounded-xl border mt-3 ${isDark ? "bg-[#2a2a2a] border-gray-700" : "bg-white border-gray-200"}`}>
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className={isDark ? "text-gray-200" : "text-gray-800"} style={{ fontSize: 13 }}>
            Realtime Session
          </div>
          <div className={`mt-0.5 ${isDark ? "text-gray-500" : "text-gray-400"}`} style={{ fontSize: 11 }}>
            使用最小 WebSocket 会话链做 controller/agent 调试，不直接进入完整媒体协商。
          </div>
        </div>
        <div className="flex gap-2">
          <button
            onClick={onRefreshEvents}
            disabled={loading || handle === null}
            className={`px-3 py-1.5 rounded-lg border transition-colors ${
              isDark
                ? "border-gray-600 text-gray-300 hover:bg-gray-800 disabled:opacity-50"
                : "border-gray-200 text-gray-700 hover:bg-gray-50 disabled:opacity-50"
            }`}
            style={{ fontSize: 12 }}
          >
            拉取事件
          </button>
          <button
            onClick={onSyncSnapshot}
            disabled={loading || handle === null}
            className={`px-3 py-1.5 rounded-lg border transition-colors ${
              isDark
                ? "border-gray-600 text-gray-300 hover:bg-gray-800 disabled:opacity-50"
                : "border-gray-200 text-gray-700 hover:bg-gray-50 disabled:opacity-50"
            }`}
            style={{ fontSize: 12 }}
          >
            同步快照
          </button>
        </div>
      </div>

      <div className="grid grid-cols-3 gap-3 mt-3">
        <label className="flex flex-col gap-1">
          <span className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 11 }}>
            本地 Device ID
          </span>
          <input
            value={deviceId}
            onChange={(event) => onDeviceIdChange(event.target.value)}
            className={`px-3 py-2 rounded-lg border outline-none ${
              isDark
                ? "bg-[#1f1f1f] border-gray-700 text-gray-100"
                : "bg-white border-gray-200 text-gray-800"
            }`}
            style={{ fontSize: 12 }}
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 11 }}>
            Session ID
          </span>
          <input
            value={sessionId}
            onChange={(event) => onSessionIdChange(event.target.value)}
            className={`px-3 py-2 rounded-lg border outline-none ${
              isDark
                ? "bg-[#1f1f1f] border-gray-700 text-gray-100"
                : "bg-white border-gray-200 text-gray-800"
            }`}
            style={{ fontSize: 12 }}
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 11 }}>
            目标 Device ID
          </span>
          <input
            value={targetDeviceId}
            onChange={(event) => onTargetDeviceIdChange(event.target.value)}
            className={`px-3 py-2 rounded-lg border outline-none ${
              isDark
                ? "bg-[#1f1f1f] border-gray-700 text-gray-100"
                : "bg-white border-gray-200 text-gray-800"
            }`}
            style={{ fontSize: 12 }}
          />
        </label>
      </div>

      <div className="grid grid-cols-2 gap-3 mt-3">
        <RealtimeSessionMetric label="连接句柄" value={handle === null ? "-" : String(handle)} />
        <RealtimeSessionMetric label="最近事件数" value={String(events.length)} />
      </div>

      <div
        className={`mt-3 rounded-lg border px-3 py-2 ${isDark ? "border-gray-700 bg-[#1f1f1f]" : "border-gray-200 bg-gray-50"}`}
        style={{ fontSize: 12 }}
      >
        <div className={isDark ? "text-gray-300" : "text-gray-700"} style={{ fontSize: 12 }}>
          协商快照
        </div>
        <div className="grid grid-cols-2 gap-3 mt-2">
          <RealtimeSessionMetric label="本地 Offer" value={snapshotLocalOffer || "-"} />
          <RealtimeSessionMetric label="远端 Offer" value={snapshotRemoteOffer || "-"} />
          <RealtimeSessionMetric label="远端 Answer" value={snapshotRemoteAnswer || "-"} />
          <RealtimeSessionMetric label="远端 ICE 数" value={String(snapshotRemoteIceCount)} />
        </div>
      </div>

      {error && (
        <div
          className={`mt-3 rounded-lg px-3 py-2 ${isDark ? "bg-red-900/20 text-red-300" : "bg-red-50 text-red-600"}`}
          style={{ fontSize: 12 }}
        >
          {error}
        </div>
      )}

      <div className="flex gap-2 mt-3">
        <RealtimeActionButton isDark={isDark} disabled={loading} onClick={onRegister}>
          注册
        </RealtimeActionButton>
        <RealtimeActionButton isDark={isDark} disabled={loading || handle === null} onClick={onRequest}>
          发起请求
        </RealtimeActionButton>
        <RealtimeActionButton isDark={isDark} disabled={loading || handle === null} onClick={onAccept}>
          接受请求
        </RealtimeActionButton>
      </div>

      <div className="grid grid-cols-2 gap-3 mt-3">
        <label className="flex flex-col gap-1">
          <span className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 11 }}>
            Offer SDP
          </span>
          <textarea
            value={offerSdp}
            onChange={(event) => onOfferSdpChange(event.target.value)}
            className={`px-3 py-2 rounded-lg border outline-none min-h-24 ${
              isDark
                ? "bg-[#1f1f1f] border-gray-700 text-gray-100"
                : "bg-white border-gray-200 text-gray-800"
            }`}
            style={{ fontSize: 12 }}
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 11 }}>
            Answer SDP
          </span>
          <textarea
            value={answerSdp}
            onChange={(event) => onAnswerSdpChange(event.target.value)}
            className={`px-3 py-2 rounded-lg border outline-none min-h-24 ${
              isDark
                ? "bg-[#1f1f1f] border-gray-700 text-gray-100"
                : "bg-white border-gray-200 text-gray-800"
            }`}
            style={{ fontSize: 12 }}
          />
        </label>
      </div>

      <div className="grid grid-cols-3 gap-3 mt-3">
        <label className="flex flex-col gap-1">
          <span className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 11 }}>
            ICE Candidate
          </span>
          <textarea
            value={iceCandidate}
            onChange={(event) => onIceCandidateChange(event.target.value)}
            className={`px-3 py-2 rounded-lg border outline-none min-h-24 ${
              isDark
                ? "bg-[#1f1f1f] border-gray-700 text-gray-100"
                : "bg-white border-gray-200 text-gray-800"
            }`}
            style={{ fontSize: 12 }}
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 11 }}>
            ICE Mid
          </span>
          <input
            value={iceSdpMid}
            onChange={(event) => onIceSdpMidChange(event.target.value)}
            className={`px-3 py-2 rounded-lg border outline-none ${
              isDark
                ? "bg-[#1f1f1f] border-gray-700 text-gray-100"
                : "bg-white border-gray-200 text-gray-800"
            }`}
            style={{ fontSize: 12 }}
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className={isDark ? "text-gray-400" : "text-gray-500"} style={{ fontSize: 11 }}>
            ICE MLine
          </span>
          <input
            type="number"
            value={iceSdpMlineIndex}
            onChange={(event) => onIceSdpMlineIndexChange(Number(event.target.value))}
            className={`px-3 py-2 rounded-lg border outline-none ${
              isDark
                ? "bg-[#1f1f1f] border-gray-700 text-gray-100"
                : "bg-white border-gray-200 text-gray-800"
            }`}
            style={{ fontSize: 12 }}
          />
        </label>
      </div>

      <div className="flex gap-2 mt-3">
        <RealtimeActionButton isDark={isDark} disabled={loading || handle === null} onClick={onSendOffer}>
          发送 Offer
        </RealtimeActionButton>
        <RealtimeActionButton isDark={isDark} disabled={loading || handle === null} onClick={onSendAnswer}>
          发送 Answer
        </RealtimeActionButton>
        <RealtimeActionButton isDark={isDark} disabled={loading || handle === null} onClick={onSendIceCandidate}>
          发送 ICE
        </RealtimeActionButton>
      </div>

      <div
        className={`mt-3 rounded-lg border px-3 py-2 ${isDark ? "border-gray-700 bg-[#1f1f1f]" : "border-gray-200 bg-gray-50"}`}
        style={{ fontSize: 12 }}
      >
        <div className={isDark ? "text-gray-300" : "text-gray-700"} style={{ fontSize: 12 }}>
          最近事件
        </div>
        <div className={`mt-2 space-y-1 max-h-32 overflow-y-auto ${isDark ? "text-gray-400" : "text-gray-500"}`}>
          {events.length === 0 ? (
            <div>暂无事件</div>
          ) : (
            events.map((event, index) => (
              <pre key={`${index}-${event}`} className="whitespace-pre-wrap break-all font-mono">
                {event}
              </pre>
            ))
          )}
        </div>
      </div>
    </div>
  );
}

function RealtimeSessionMetric({ label, value }: { label: string; value: string }) {
  const { isDark } = useTheme();

  return (
    <div className={`rounded-lg border px-3 py-2 ${isDark ? "border-gray-700 bg-[#1f1f1f]" : "border-gray-200 bg-gray-50"}`}>
      <div className={isDark ? "text-gray-500" : "text-gray-400"} style={{ fontSize: 11 }}>
        {label}
      </div>
      <div className={isDark ? "text-gray-100" : "text-gray-800"} style={{ fontSize: 13 }}>
        {value}
      </div>
    </div>
  );
}

function RealtimeActionButton({
  isDark,
  disabled,
  onClick,
  children,
}: {
  isDark: boolean;
  disabled: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={`px-3 py-1.5 rounded-lg border transition-colors ${
        isDark
          ? "border-gray-600 text-gray-300 hover:bg-gray-800 disabled:opacity-50"
          : "border-gray-200 text-gray-700 hover:bg-gray-50 disabled:opacity-50"
      }`}
      style={{ fontSize: 12 }}
    >
      {children}
    </button>
  );
}
