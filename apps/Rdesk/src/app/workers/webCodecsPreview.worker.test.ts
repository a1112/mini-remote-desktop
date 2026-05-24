import { describe, expect, it } from "vitest";
import { buildWebCodecsPreviewStartControlMessage } from "./webCodecsPreview.worker";

describe("webCodecsPreview worker", () => {
  it("includes selected source id in the WebSocket start control message", () => {
    expect(
      buildWebCodecsPreviewStartControlMessage({
        sessionId: "local-display-test-1",
        fps: 120,
        width: 3840,
        height: 2160,
        bitrateMbps: 80,
        h264Profile: "baseline",
        sourceId: "windows:display-shared:1",
      })
    ).toEqual({
      type: "start",
      session_id: "local-display-test-1",
      fps: 120,
      width: 3840,
      height: 2160,
      bitrate_mbps: 80,
      h264_profile: "baseline",
      source_id: "windows:display-shared:1",
    });
  });
});
