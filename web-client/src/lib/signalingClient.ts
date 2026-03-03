import type { DeviceInfo, SignalEnvelope } from "./types";

type Handler<T> = (event: T) => void;

type SignalEvents = {
  status: "offline" | "connecting" | "online";
  selfId: string;
  deviceList: DeviceInfo[];
  offline: string;
  webrtc: SignalEnvelope;
  log: string;
};

type OfferOptions = {
  transport?: "webrtc" | "quic";
  capabilities?: Record<string, unknown>;
};

function buildWebClientCapabilities(): Record<string, unknown> {
  return {
    protocols: ["webrtc"],
    platforms: ["web"],
    codecs: ["h264"],
    features: ["multi-end-compat", "capability-negotiation"]
  };
}

export class SignalingClient {
  private readonly wsUrl: string;
  private ws: WebSocket | null = null;
  private reconnectMs = 1500;
  private closedByUser = false;
  private handlers: { [K in keyof SignalEvents]: Handler<SignalEvents[K]>[] } = {
    status: [],
    selfId: [],
    deviceList: [],
    offline: [],
    webrtc: [],
    log: []
  };

  constructor(wsUrl: string) {
    this.wsUrl = wsUrl;
  }

  on<K extends keyof SignalEvents>(event: K, handler: Handler<SignalEvents[K]>): () => void {
    this.handlers[event].push(handler);
    return () => {
      const arr = this.handlers[event] as Handler<SignalEvents[K]>[];
      const index = arr.indexOf(handler);
      if (index >= 0) {
        arr.splice(index, 1);
      }
    };
  }

  connect(): void {
    this.closedByUser = false;
    this.emit("status", "connecting");
    this.ws = new WebSocket(this.wsUrl);

    this.ws.onopen = () => {
      this.emit("log", `WS connected: ${this.wsUrl}`);
      this.reconnectMs = 1500;
    };
    this.ws.onclose = () => {
      this.emit("status", "offline");
      this.emit("log", "WS closed");
      if (!this.closedByUser) {
        window.setTimeout(() => this.connect(), this.reconnectMs);
        this.reconnectMs = Math.min(this.reconnectMs * 2, 8000);
      }
    };
    this.ws.onerror = () => {
      this.emit("log", "WS error");
    };
    this.ws.onmessage = (event) => this.handleMessage(event.data);
  }

  close(): void {
    this.closedByUser = true;
    this.ws?.close();
    this.ws = null;
  }

  requestDeviceList(): void {
    this.send({ type: "device", action: "getDeviceList" });
  }

  sendOffer(targetDeviceId: string, offer: RTCSessionDescriptionInit, options?: OfferOptions): void {
    this.send({
      type: "webrtc",
      action: "offer",
      payload: {
        targetDeviceId,
        offer,
        transport: options?.transport ?? "webrtc",
        capabilities: options?.capabilities ?? buildWebClientCapabilities()
      }
    });
  }

  sendAnswer(controllerId: string, answer: RTCSessionDescriptionInit): void {
    this.send({ type: "webrtc", action: "answer", payload: { controllerId, answer } });
  }

  sendIceCandidate(targetDeviceId: string, candidate: RTCIceCandidateInit): void {
    this.send({ type: "webrtc", action: "iceCandidate", payload: { targetDeviceId, candidate } });
  }

  private send(message: SignalEnvelope): void {
    if (this.ws?.readyState !== WebSocket.OPEN) {
      return;
    }
    this.ws.send(JSON.stringify(message));
  }

  private handleMessage(raw: unknown): void {
    if (typeof raw !== "string") {
      return;
    }
    let message: SignalEnvelope;
    try {
      message = JSON.parse(raw) as SignalEnvelope;
    } catch {
      this.emit("log", "Invalid JSON from signaling");
      return;
    }

    if (message.type === "system" && message.action === "connected") {
      const selfId = String((message.payload?.deviceId as string) ?? "");
      if (selfId) {
        this.emit("selfId", selfId);
      }
      this.send({
        type: "device",
        action: "register",
        payload: {
          type: "controller",
          name: "Web Client",
          protocolVersion: 2,
          transports: ["webrtc"],
          capabilities: buildWebClientCapabilities()
        }
      });
      return;
    }

    if (message.type === "device" && message.action === "registered") {
      const list = ((message.payload?.deviceList ?? []) as DeviceInfo[]) ?? [];
      this.emit("status", "online");
      this.emit("deviceList", list);
      return;
    }

    if (message.type === "device" && message.action === "deviceList") {
      const list = ((message.payload?.deviceList ?? []) as DeviceInfo[]) ?? [];
      this.emit("deviceList", list);
      return;
    }

    if (message.type === "device" && message.action === "offline") {
      const offlineId = String((message.payload?.deviceId as string) ?? "");
      if (offlineId) {
        this.emit("offline", offlineId);
      }
      return;
    }

    if (message.type === "webrtc") {
      this.emit("webrtc", message);
    }
  }

  private emit<K extends keyof SignalEvents>(event: K, payload: SignalEvents[K]): void {
    for (const handler of this.handlers[event]) {
      handler(payload);
    }
  }
}
