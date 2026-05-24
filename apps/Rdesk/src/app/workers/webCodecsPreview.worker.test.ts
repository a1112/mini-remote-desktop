import { describe, expect, it } from "vitest";
import {
  buildWebCodecsPreviewStartControlMessage,
  buildWebCodecsVideoDecoderConfig,
} from "./webCodecsPreview.worker";

describe("webCodecsPreview worker", () => {
  it("includes selected source id in the WebSocket start control message", () => {
    expect(
      buildWebCodecsPreviewStartControlMessage({
        sessionId: "local-display-test-1",
        fps: 120,
        width: 3840,
        height: 2160,
        bitrateMbps: 80,
        codec: "h264",
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
      codec: "h264",
      h264_profile: "baseline",
      source_id: "windows:display-shared:1",
    });
  });

  it("includes HEVC codec selection in the WebSocket start control message", () => {
    expect(
      buildWebCodecsPreviewStartControlMessage({
        sessionId: "local-display-test-1",
        fps: 120,
        width: 2560,
        height: 1440,
        bitrateMbps: 40,
        codec: "hevc",
        h264Profile: "baseline",
      })
    ).toMatchObject({
      type: "start",
      codec: "hevc",
      h264_profile: "baseline",
      source_id: null,
    });
  });

  it("builds HEVC decoder config without H.264 avc metadata", () => {
    const config = buildWebCodecsVideoDecoderConfig({
      type: "mrd.webcodecs.ready.v1",
      session_id: "s1",
      codec: "hev1.1.6.L156.B0",
      codec_format: "annexb",
      width: 2560,
      height: 1440,
      fps: 120,
      bitrate_mbps: 40,
    });

    expect(config).toMatchObject({
      codec: "hev1.1.6.L156.B0",
      hevc: { format: "annexb" },
    });
    expect("avc" in config).toBe(false);
  });
});
