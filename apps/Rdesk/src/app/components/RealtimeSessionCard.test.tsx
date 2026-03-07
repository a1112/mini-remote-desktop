import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("./ThemeContext", () => ({
  useTheme: () => ({
    isDark: false,
  }),
}));

describe("RealtimeSessionCard", () => {
  it("renders registration state and recent events", async () => {
    const { RealtimeSessionCard } = await import("./RealtimeSessionCard");

    const html = renderToStaticMarkup(
      <RealtimeSessionCard
        deviceId="controller-1"
        sessionId="session-1"
        targetDeviceId="agent-1"
        handle={7}
        offerSdp="offer-sdp"
        answerSdp="answer-sdp"
        iceCandidate="candidate:1 1 UDP 123 127.0.0.1 5000 typ host"
        iceSdpMid="0"
        iceSdpMlineIndex={0}
        snapshotLocalOffer="offer-sdp"
        snapshotRemoteOffer="remote-offer-sdp"
        snapshotRemoteAnswer="answer-sdp"
        snapshotRemoteIceCount={1}
        hostLocalOffer="host-offer-sdp"
        hostRemoteOffer="host-remote-offer-sdp"
        hostLocalAnswer="host-answer-sdp"
        hostRemoteAnswer="host-remote-answer-sdp"
        hostRemoteIceCount={2}
        hostRemoteVideoTrackCount={1}
        hostRemoteRtpPacketCount={42}
        hostLastRemoteCodec="video/h264"
        hostRemoteH264AccessUnitCount={5}
        hostLastRemoteAccessUnitBytes={1200}
        loading={false}
        error={null}
        events={[
          "{\"type\":\"session\",\"action\":\"request\"}",
          "{\"type\":\"session\",\"action\":\"accept\"}",
        ]}
        onDeviceIdChange={() => {}}
        onSessionIdChange={() => {}}
        onTargetDeviceIdChange={() => {}}
        onOfferSdpChange={() => {}}
        onAnswerSdpChange={() => {}}
        onIceCandidateChange={() => {}}
        onIceSdpMidChange={() => {}}
        onIceSdpMlineIndexChange={() => {}}
        onRegister={() => {}}
        onRequest={() => {}}
        onAccept={() => {}}
        onSendOffer={() => {}}
        onSendAnswer={() => {}}
        onSendIceCandidate={() => {}}
        onRefreshEvents={() => {}}
        onSyncSnapshot={() => {}}
      />
    );

    expect(html).toContain("Realtime Session");
    expect(html).toContain("controller-1");
    expect(html).toContain("session-1");
    expect(html).toContain("request");
    expect(html).toContain("accept");
    expect(html).toContain("Offer SDP");
    expect(html).toContain("Answer SDP");
    expect(html).toContain("ICE Candidate");
    expect(html).toContain("生成 Offer 并发送");
    expect(html).toContain("生成 Answer 并发送");
    expect(html).toContain("应用并发送 ICE");
    expect(html).toContain("同步快照");
    expect(html).toContain("协商快照");
    expect(html).toContain("remote-offer-sdp");
    expect(html).toContain("Native Host 快照");
    expect(html).toContain("host-answer-sdp");
    expect(html).toContain("video/h264");
    expect(html).toContain("42");
  });
});
