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
        loading={false}
        error={null}
        events={[
          "{\"type\":\"session\",\"action\":\"request\"}",
          "{\"type\":\"session\",\"action\":\"accept\"}",
        ]}
        onDeviceIdChange={() => {}}
        onSessionIdChange={() => {}}
        onTargetDeviceIdChange={() => {}}
        onRegister={() => {}}
        onRequest={() => {}}
        onAccept={() => {}}
        onRefreshEvents={() => {}}
      />
    );

    expect(html).toContain("Realtime Session");
    expect(html).toContain("controller-1");
    expect(html).toContain("session-1");
    expect(html).toContain("request");
    expect(html).toContain("accept");
  });
});
