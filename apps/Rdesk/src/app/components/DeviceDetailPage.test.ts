import { describe, expect, it } from "vitest";

import {
  deviceDetailTabFromSearch,
  remoteStartUnavailableReason,
  remoteApplicationSourceMatchesTerminalFocus,
} from "./DeviceDetailPage";
import type { Device } from "./deviceData";

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

describe("remoteStartUnavailableReason", () => {
  const device = (overrides: Partial<Device> = {}): Device => ({
    id: "remote-device",
    name: "Remote PC",
    deviceId: "remote-device",
    os: "Windows",
    icon: (() => null) as unknown as Device["icon"],
    status: "online",
    location: "LAN",
    ping: 7,
    lastSeen: "刚刚",
    cpu: null,
    ram: null,
    disk: null,
    ip: "192.168.1.2",
    group: "LAN P2P",
    favorite: false,
    discoverySources: ["lan_p2p"],
    primarySource: "lan_p2p",
    sourceLabel: "P2P 局域网",
    isLocal: false,
    p2pAvailable: true,
    serverAvailable: false,
    ...overrides,
  });

  it("blocks remote start for disabled devices", () => {
    expect(remoteStartUnavailableReason(device({ disabled: true }))).toBe("设备已禁用");
  });

  it("allows online enabled devices to start remote control", () => {
    expect(remoteStartUnavailableReason(device())).toBeNull();
  });
});
