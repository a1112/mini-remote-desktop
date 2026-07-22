export class WebRtcMediaClient {
  constructor({ iceServers, onIceCandidate, onTrack, onConnectionState, onIceState, onDataChannelMessage, onDataChannelOpen }) {
    this.pc = new RTCPeerConnection({
      iceServers,
      rtcConfiguration: {
        iceTransportPolicy: 'all',
        bundlePolicy: 'max-bundle',
        rtcpMuxPolicy: 'require'
      }
    });
    this.dataChannel = null;
    this.pc.onicecandidate = (event) => onIceCandidate?.(event.candidate);
    this.pc.ontrack = (event) => onTrack?.(event.streams[0]);
    this.pc.onconnectionstatechange = () => onConnectionState?.(this.pc.connectionState);
    this.pc.oniceconnectionstatechange = () => onIceState?.(this.pc.iceConnectionState);
    this.onDataChannelMessage = onDataChannelMessage;
    this.onDataChannelOpen = onDataChannelOpen;
  }

  createControlChannel() {
    this.dataChannel = this.pc.createDataChannel('control', { ordered: false, maxRetransmits: 0 });
    this.dataChannel.onopen = () => this.onDataChannelOpen?.();
    this.dataChannel.onmessage = (event) => this.onDataChannelMessage?.(event.data);
  }

  async createOffer() {
    const offer = await this.pc.createOffer({ offerToReceiveAudio: false, offerToReceiveVideo: true });
    await this.pc.setLocalDescription(offer);
    return offer;
  }

  async setRemoteAnswer(answer) {
    await this.pc.setRemoteDescription(new RTCSessionDescription(answer));
  }

  async addIceCandidate(candidate) {
    if (!candidate || !candidate.candidate) return;
    await this.pc.addIceCandidate(new RTCIceCandidate(candidate));
  }

  isControlReady() {
    return this.dataChannel?.readyState === 'open';
  }

  sendControl(payload) {
    if (!this.isControlReady()) return false;
    this.dataChannel.send(JSON.stringify(payload));
    return true;
  }

  close() {
    try {
      this.dataChannel?.close();
    } catch (_) {}
    try {
      this.pc?.close();
    } catch (_) {}
    this.dataChannel = null;
    this.pc = null;
  }
}

