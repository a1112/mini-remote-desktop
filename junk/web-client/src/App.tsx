import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { DeviceStore } from "./lib/deviceStore";
import { SignalingClient } from "./lib/signalingClient";
import type { AppLog, DeviceInfo } from "./lib/types";
import { WebRTCSession } from "./lib/webrtcSession";
import { buildWsUrl } from "./lib/ws-url";

type WsState = "offline" | "connecting" | "online";

type RuntimeStats = {
  bitrateMbps: string;
  fps: string;
  rttMs: string;
  packetsLost: string;
  bytesReceived: string;
  framesDecoded: string;
  packetsReceived: string;
};

const INITIAL_STATS: RuntimeStats = {
  bitrateMbps: "-",
  fps: "-",
  rttMs: "-",
  packetsLost: "-",
  bytesReceived: "-",
  framesDecoded: "-",
  packetsReceived: "-"
};

function nowLog(level: AppLog["level"], message: string): AppLog {
  return { ts: Date.now(), level, message };
}

export default function App() {
  const wsUrl = useMemo(() => buildWsUrl(window.location), []);
  const [wsState, setWsState] = useState<WsState>("offline");
  const [peerState, setPeerState] = useState<string>("idle");
  const [selfId, setSelfId] = useState<string>("-");
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [selectedDeviceId, setSelectedDeviceId] = useState<string>("");
  const [logs, setLogs] = useState<AppLog[]>([]);
  const [stats, setStats] = useState<RuntimeStats>(INITIAL_STATS);
  const videoRef = useRef<HTMLVideoElement>(null);
  const signalingRef = useRef<SignalingClient | null>(null);
  const sessionRef = useRef<WebRTCSession | null>(null);
  const deviceStoreRef = useRef(new DeviceStore(15_000));
  const metricsRef = useRef({ lastBytes: 0, lastFramesDecoded: 0, lastTs: 0 });
  const connectTargetRef = useRef<string>("");

  useEffect(() => {
    const signaling = new SignalingClient(wsUrl);
    signalingRef.current = signaling;

    const log = (line: string, level: AppLog["level"] = "info") => {
      setLogs((prev) => [nowLog(level, line), ...prev].slice(0, 200));
    };

    const unsubs = [
      signaling.on("status", (status) => setWsState(status)),
      signaling.on("selfId", (id) => setSelfId(id)),
      signaling.on("log", (line) => log(line)),
      signaling.on("deviceList", (list) => {
        deviceStoreRef.current.upsertMany(list);
        setDevices(deviceStoreRef.current.list());
      }),
      signaling.on("offline", (deviceId) => {
        deviceStoreRef.current.markOffline(deviceId);
        setDevices(deviceStoreRef.current.list());
        if (connectTargetRef.current && connectTargetRef.current === deviceId) {
          disconnect();
        }
      }),
      signaling.on("webrtc", async (msg) => {
        try {
          if (msg.action === "answer" && msg.payload?.answer && sessionRef.current) {
            await sessionRef.current.setRemoteAnswer(msg.payload.answer as RTCSessionDescriptionInit);
            log("Received answer");
            return;
          }
          if (msg.action === "iceCandidate" && msg.payload?.candidate && sessionRef.current) {
            await sessionRef.current.addRemoteIce(msg.payload.candidate as RTCIceCandidateInit);
          }
        } catch (err) {
          log(`WebRTC message error: ${String(err)}`, "error");
        }
      })
    ];

    signaling.connect();
    const timer = window.setInterval(() => {
      signaling.requestDeviceList();
      deviceStoreRef.current.prune();
      setDevices(deviceStoreRef.current.list());
    }, 1000);

    return () => {
      window.clearInterval(timer);
      for (const off of unsubs) {
        off();
      }
      disconnect();
      signaling.close();
    };
  }, [wsUrl]);

  useEffect(() => {
    const timer = window.setInterval(async () => {
      if (!sessionRef.current) {
        return;
      }
      const current = await sessionRef.current.getInboundVideoStats();
      if (!current) {
        return;
      }
      const now = Date.now();
      const deltaBytes = current.bytesReceived - metricsRef.current.lastBytes;
      const deltaMs = now - metricsRef.current.lastTs;
      const bitrate = deltaMs > 0 ? ((deltaBytes * 8) / (deltaMs / 1000) / 1_000_000).toFixed(2) : "-";
      const deltaFrames = current.framesDecoded - metricsRef.current.lastFramesDecoded;
      const fpsFallback = deltaMs > 0 ? (deltaFrames * 1000) / deltaMs : 0;
      metricsRef.current.lastBytes = current.bytesReceived;
      metricsRef.current.lastFramesDecoded = current.framesDecoded;
      metricsRef.current.lastTs = now;
      setStats({
        bitrateMbps: bitrate,
        fps:
          current.fps > 0
            ? current.fps.toFixed(0)
            : fpsFallback > 0
              ? fpsFallback.toFixed(0)
              : "-",
        rttMs: current.rttMs > 0 ? current.rttMs.toFixed(0) : "-",
        packetsLost: String(current.packetsLost),
        bytesReceived: String(current.bytesReceived),
        framesDecoded: String(current.framesDecoded),
        packetsReceived: String(current.packetsReceived)
      });
    }, 1000);
    return () => window.clearInterval(timer);
  }, []);

  async function connect(): Promise<void> {
    if (!selectedDeviceId || !signalingRef.current) {
      return;
    }
    disconnect();
    connectTargetRef.current = selectedDeviceId;
    metricsRef.current = { lastBytes: 0, lastFramesDecoded: 0, lastTs: 0 };

    const session = new WebRTCSession({
      onLocalIce: (candidate) => signalingRef.current?.sendIceCandidate(selectedDeviceId, candidate),
      onRemoteStream: (stream) => {
        if (videoRef.current) {
          videoRef.current.srcObject = stream;
          videoRef.current
            .play()
            .then(() => {
              setLogs((prev) => [nowLog("info", "Video play started"), ...prev].slice(0, 200));
            })
            .catch((err) => {
              setLogs((prev) =>
                [nowLog("warn", `Video play blocked: ${String(err)}`), ...prev].slice(0, 200)
              );
            });
        }
      },
      onStateChange: (state) => setPeerState(state),
      onLog: (line) => setLogs((prev) => [nowLog("info", line), ...prev].slice(0, 200))
    });
    sessionRef.current = session;

    try {
      const offer = await session.createOffer();
      signalingRef.current.sendOffer(selectedDeviceId, offer);
      setLogs((prev) => [nowLog("info", `Offer sent to ${selectedDeviceId}`), ...prev].slice(0, 200));
    } catch (err) {
      setLogs((prev) => [nowLog("error", `Create offer failed: ${String(err)}`), ...prev].slice(0, 200));
    }
  }

  function disconnect(): void {
    sessionRef.current?.close();
    sessionRef.current = null;
    connectTargetRef.current = "";
    setPeerState("idle");
    setStats(INITIAL_STATS);
    if (videoRef.current) {
      videoRef.current.srcObject = null;
    }
  }

  function toVideoCoords(e: MouseEvent<HTMLVideoElement>): { x: number; y: number } {
    const video = e.currentTarget;
    const rect = video.getBoundingClientRect();
    const x = (e.clientX - rect.left) * ((video.videoWidth || rect.width) / rect.width);
    const y = (e.clientY - rect.top) * ((video.videoHeight || rect.height) / rect.height);
    return { x, y };
  }

  return (
    <div className="app">
      <header className="panel">
        <h1>Mini Remote Desktop Web Client</h1>
        <div className="meta">
          <span>WS: {wsState}</span>
          <span>Peer: {peerState}</span>
          <span>Self: {selfId}</span>
          <span>Signal: {wsUrl}</span>
        </div>
      </header>

      <main className="grid">
        <section className="panel">
          <h2>设备</h2>
          <div className="list">
            {devices.map((d) => (
              <button
                key={d.id}
                className={`device ${selectedDeviceId === d.id ? "active" : ""}`}
                onClick={() => setSelectedDeviceId(d.id)}
              >
                <span>{d.name}</span>
                <small>{d.online ? "online" : "offline"}</small>
              </button>
            ))}
            {devices.length === 0 && <p className="empty">暂无在线 agent</p>}
          </div>
          <div className="actions">
            <button onClick={connect} disabled={!selectedDeviceId || wsState !== "online"}>
              连接
            </button>
            <button onClick={disconnect}>断开</button>
            <button onClick={() => signalingRef.current?.requestDeviceList()}>刷新</button>
          </div>
        </section>

        <section className="panel">
          <h2>画面</h2>
          <video
            ref={videoRef}
            autoPlay
            muted
            playsInline
            tabIndex={0}
            onLoadedMetadata={() =>
              setLogs((prev) => [nowLog("info", "Video metadata loaded"), ...prev].slice(0, 200))
            }
            onCanPlay={() =>
              setLogs((prev) => [nowLog("info", "Video canplay"), ...prev].slice(0, 200))
            }
            onMouseMove={(e) => {
              const p = toVideoCoords(e);
              sessionRef.current?.sendMouse("move", p.x, p.y);
            }}
            onMouseDown={(e) => {
              const p = toVideoCoords(e);
              sessionRef.current?.sendMouse("down", p.x, p.y, e.button);
            }}
            onMouseUp={(e) => {
              const p = toVideoCoords(e);
              sessionRef.current?.sendMouse("up", p.x, p.y, e.button);
            }}
            onKeyDown={(e) => {
              e.preventDefault();
              sessionRef.current?.sendKeyboard("down", e.key, e.code);
            }}
            onKeyUp={(e) => {
              e.preventDefault();
              sessionRef.current?.sendKeyboard("up", e.key, e.code);
            }}
          />
          <div className="stats">
            <span>Bitrate: {stats.bitrateMbps} Mbps</span>
            <span>FPS: {stats.fps}</span>
            <span>RTT: {stats.rttMs} ms</span>
            <span>Loss: {stats.packetsLost}</span>
          </div>
          <div className="stats">
            <span>Inbound bytesReceived: {stats.bytesReceived}</span>
            <span>Inbound framesDecoded: {stats.framesDecoded}</span>
            <span>Inbound packetsReceived: {stats.packetsReceived}</span>
            <span>Inbound packetsLost: {stats.packetsLost}</span>
          </div>
        </section>

        <section className="panel">
          <h2>信令日志</h2>
          <div className="logs">
            {logs.map((log) => (
              <div key={`${log.ts}-${log.message}`} className={`log-${log.level}`}>
                [{new Date(log.ts).toLocaleTimeString()}] {log.message}
              </div>
            ))}
          </div>
        </section>
      </main>
    </div>
  );
}
