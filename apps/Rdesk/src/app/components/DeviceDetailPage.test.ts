import { describe, expect, it } from "vitest";

import {
  deviceDetailTabFromSearch,
  fileTransferDropRequestForSendToOther,
  fileTransferEntriesFromSelection,
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

describe("file transfer send-to-other helpers", () => {
  const files = [
    {
      name: "notes.txt",
      path: "G:\\Project\\notes.txt",
      kind: "file" as const,
      type: "file" as const,
      size: "4 KB",
      modified: "2026-03-04",
      fileKind: "文本文件",
    },
    {
      name: "Screens",
      path: "G:\\Project\\Screens",
      kind: "directory" as const,
      type: "folder" as const,
      size: "--",
      modified: "2026-03-04",
      fileKind: "文件夹",
    },
    {
      name: "fallback-only.txt",
      type: "file" as const,
      size: "1 KB",
      modified: "2026-03-04",
      fileKind: "文本文件",
    },
  ];

  it("builds path-backed entries from the selected context file set", () => {
    expect(
      fileTransferEntriesFromSelection(files, ["notes.txt", "Screens"], "notes.txt")
    ).toEqual([
      {
        source_path: "G:\\Project\\notes.txt",
        file_name: "notes.txt",
        kind: "file",
      },
      {
        source_path: "G:\\Project\\Screens",
        file_name: "Screens",
        kind: "directory",
      },
    ]);
  });

  it("falls back to the context file when it is not part of the selection", () => {
    expect(
      fileTransferEntriesFromSelection(files, ["notes.txt"], "Screens")
    ).toEqual([
      {
        source_path: "G:\\Project\\Screens",
        file_name: "Screens",
        kind: "directory",
      },
    ]);
  });

  it("creates a drop request for the other pane current service path", () => {
    expect(
      fileTransferDropRequestForSendToOther({
        sourceDeviceId: "left-device",
        targetDeviceId: "right-device",
        targetPath: "G:\\Target",
        files,
        selectedNames: ["notes.txt"],
        contextFileName: "notes.txt",
      })
    ).toEqual({
      sourceDeviceId: "left-device",
      targetDeviceId: "right-device",
      targetPath: "G:\\Target",
      entries: [
        {
          source_path: "G:\\Project\\notes.txt",
          file_name: "notes.txt",
          kind: "file",
        },
      ],
    });
  });

  it("does not create a request without a target path or path-backed entries", () => {
    expect(
      fileTransferDropRequestForSendToOther({
        sourceDeviceId: "left-device",
        targetDeviceId: "right-device",
        targetPath: null,
        files,
        selectedNames: ["notes.txt"],
        contextFileName: "notes.txt",
      })
    ).toBeNull();

    expect(
      fileTransferDropRequestForSendToOther({
        sourceDeviceId: "left-device",
        targetDeviceId: "right-device",
        targetPath: "G:\\Target",
        files,
        selectedNames: ["fallback-only.txt"],
        contextFileName: "fallback-only.txt",
      })
    ).toBeNull();
  });
});
