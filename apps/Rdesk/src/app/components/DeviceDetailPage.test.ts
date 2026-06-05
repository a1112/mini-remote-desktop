import { describe, expect, it } from "vitest";

import {
  deviceDetailTabFromSearch,
  remoteApplicationSourceMatchesTerminalFocus,
} from "./DeviceDetailPage";

describe("deviceDetailTabFromSearch", () => {
  it("opens supported tabs from the sidebar query string", () => {
    expect(deviceDetailTabFromSearch("?tab=files")).toBe("files");
    expect(deviceDetailTabFromSearch("?tab=apps")).toBe("apps");
    expect(deviceDetailTabFromSearch("?tab=terminal")).toBe("terminal");
    expect(deviceDetailTabFromSearch("?tab=info")).toBe("info");
  });

  it("falls back to the remote tab for unsupported values", () => {
    expect(deviceDetailTabFromSearch("")).toBe("remote");
    expect(deviceDetailTabFromSearch("?tab=unknown")).toBe("remote");
  });
});

describe("remoteApplicationSourceMatchesTerminalFocus", () => {
  it("matches common terminal window names", () => {
    expect(
      remoteApplicationSourceMatchesTerminalFocus({
        app_name: "Windows Terminal",
        title: "Administrator: PowerShell",
      })
    ).toBe(true);
    expect(
      remoteApplicationSourceMatchesTerminalFocus({
        app_name: "cmd.exe",
        title: "Command Prompt",
      })
    ).toBe(true);
  });

  it("does not match non-terminal application windows", () => {
    expect(
      remoteApplicationSourceMatchesTerminalFocus({
        app_name: "Notepad",
        title: "notes.txt",
      })
    ).toBe(false);
  });
});
