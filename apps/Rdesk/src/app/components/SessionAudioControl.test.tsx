import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SessionAudioControl } from "./SessionAudioControl";
const api = vi.hoisted(() => ({ get: vi.fn(), mute: vi.fn() }));
vi.mock("../adapters/tauri", () => ({ ipcGetSessionAudio: api.get, ipcSetSessionAudioMuted: api.mute }));
const snapshot = { session_id: "audio-1", status: "playing", muted: false, packets: 10, dropped_packets: 0, buffered_frames: 960, underrun_packets: 0, sample_rate: 48000, channels: 2, last_error: null };
describe("session audio control", () => {
  beforeEach(() => { vi.clearAllMocks(); api.get.mockResolvedValue({ ok: true, value: snapshot }); });
  it("does not poll or offer playback without an audio grant", () => {
    render(<SessionAudioControl sessionId="audio-1" authorized={false} />);
    expect(screen.getByRole("button", { name: "声音未授权" })).toBeDisabled();
    expect(api.get).not.toHaveBeenCalled();
  });
  it("uses the service mute result", async () => {
    api.mute.mockResolvedValue({ ok: true, value: { ...snapshot, muted: true } });
    render(<SessionAudioControl sessionId="audio-1" authorized />);
    fireEvent.click(await screen.findByRole("button", { name: "远程声音" }));
    await waitFor(() => expect(api.mute).toHaveBeenCalledWith("audio-1", true));
    expect(await screen.findByRole("button", { name: "声音已静音" })).toHaveAttribute("aria-pressed", "true");
  });
  it("reports a native audio failure instead of offering a working speaker", async () => {
    api.get.mockResolvedValue({ ok: true, value: { ...snapshot, status: "failed", last_error: "no audio output device" } });
    render(<SessionAudioControl sessionId="audio-1" authorized />);
    const button = await screen.findByRole("button", { name: "声音不可用" });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("title", "no audio output device");
  });
});
