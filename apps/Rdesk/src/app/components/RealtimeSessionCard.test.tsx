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
    expect(html).toContain("发送 Offer");
    expect(html).toContain("发送 Answer");
    expect(html).toContain("发送 ICE");
    expect(html).toContain("同步快照");
    expect(html).toContain("协商快照");
    expect(html).toContain("remote-offer-sdp");
  });
});
