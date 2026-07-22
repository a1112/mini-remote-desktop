type SessionHooks = {
  onLocalIce: (candidate: RTCIceCandidateInit) => void;
  onRemoteStream: (stream: MediaStream) => void;
  onStateChange: (state: RTCPeerConnectionState) => void;
  onLog: (line: string) => void;
};

const ICE_SERVERS: RTCIceServer[] = [
  { urls: "stun:stun.l.google.com:19302" },
  { urls: "stun:stun1.l.google.com:19302" }
];

export class WebRTCSession {
  private pc: RTCPeerConnection | null = null;
  private controlDc: RTCDataChannel | null = null;
  private readonly hooks: SessionHooks;

  constructor(hooks: SessionHooks) {
    this.hooks = hooks;
  }

  async createOffer(): Promise<RTCSessionDescriptionInit> {
    this.ensurePeerConnection();
    const offer = await this.pc!.createOffer({
      offerToReceiveAudio: false,
      offerToReceiveVideo: true
    });
    await this.pc!.setLocalDescription(offer);
    return offer;
  }

  async setRemoteAnswer(answer: RTCSessionDescriptionInit): Promise<void> {
    if (!this.pc) {
      return;
    }
    await this.pc.setRemoteDescription(new RTCSessionDescription(answer));
  }

  async addRemoteIce(candidate: RTCIceCandidateInit): Promise<void> {
    if (!this.pc) {
      return;
    }
    await this.pc.addIceCandidate(new RTCIceCandidate(candidate));
  }

  sendMouse(action: "move" | "down" | "up", x: number, y: number, button = 0): void {
    if (!this.controlDc || this.controlDc.readyState !== "open") {
      return;
    }
    this.controlDc.send(
      JSON.stringify({
        type: "mouse",
        action,
        x: Math.round(x),
        y: Math.round(y),
        button
      })
    );
  }

  sendKeyboard(action: "down" | "up", key: string, code: string): void {
    if (!this.controlDc || this.controlDc.readyState !== "open") {
      return;
    }
    this.controlDc.send(JSON.stringify({ type: "keyboard", action, key, code }));
  }

  async getInboundVideoStats(): Promise<{
    bytesReceived: number;
    packetsReceived: number;
    packetsLost: number;
    fps: number;
    framesDecoded: number;
    rttMs: number;
  } | null> {
    if (!this.pc) {
      return null;
    }

    const stats = await this.pc.getStats();
    let bytes = 0;
    let packetsReceived = 0;
    let fps = 0;
    let lost = 0;
    let framesDecoded = 0;
    let rtt = 0;

    for (const report of stats.values()) {
      if (report.type === "inbound-rtp" && (report as RTCInboundRtpStreamStats).kind === "video") {
        const v = report as RTCInboundRtpStreamStats;
        bytes = v.bytesReceived ?? bytes;
        packetsReceived = v.packetsReceived ?? packetsReceived;
        fps = v.framesPerSecond ?? fps;
        lost = v.packetsLost ?? lost;
        framesDecoded = v.framesDecoded ?? framesDecoded;
      }
      if (report.type === "candidate-pair" && (report as RTCIceCandidatePairStats).state === "succeeded") {
        const cp = report as RTCIceCandidatePairStats;
        rtt = ((cp.currentRoundTripTime ?? 0) * 1000) || rtt;
      }
    }

    return {
      bytesReceived: bytes,
      packetsReceived,
      packetsLost: lost,
      fps,
      framesDecoded,
      rttMs: rtt
    };
  }

  close(): void {
    this.controlDc?.close();
    this.pc?.close();
    this.controlDc = null;
    this.pc = null;
  }

  private ensurePeerConnection(): void {
    if (this.pc) {
      return;
    }
    this.pc = new RTCPeerConnection({
      iceServers: ICE_SERVERS
    });
    this.pc.onicecandidate = (event) => {
      if (event.candidate) {
        this.hooks.onLocalIce(event.candidate.toJSON());
      }
    };
    this.pc.ontrack = (event) => {
      const stream = event.streams[0] ?? new MediaStream([event.track]);
      if (stream) {
        this.hooks.onRemoteStream(stream);
      }
    };
    this.pc.onconnectionstatechange = () => {
      const state = this.pc?.connectionState ?? "closed";
      this.hooks.onStateChange(state);
      this.hooks.onLog(`PC state: ${state}`);
    };

    this.controlDc = this.pc.createDataChannel("control", {
      ordered: false,
      maxRetransmits: 0
    });
    this.controlDc.onopen = () => this.hooks.onLog("DataChannel open");
    this.controlDc.onerror = () => this.hooks.onLog("DataChannel error");
  }
}
