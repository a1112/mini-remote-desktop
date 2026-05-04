import { describe, expect, it } from "vitest";
import { Monitor } from "lucide-react";

import { type Device, mergeDevices } from "./deviceData";

const device = (overrides: Partial<Device>): Device => ({
  id: "base-id",
  name: "Base Device",
  deviceId: "base-device-id",
  os: "Windows",
  icon: Monitor,
  status: "online",
  location: "Local",
  ping: 0,
  lastSeen: "now",
  cpu: null,
  ram: null,
  disk: null,
  ip: "127.0.0.1",
  group: "Local",
  favorite: false,
  discoverySources: ["local"],
  primarySource: "local",
  sourceLabel: "本机",
  isLocal: true,
  p2pAvailable: false,
  serverAvailable: false,
  ...overrides,
});

describe("mergeDevices", () => {
  it("keeps the computer hostname for the local device instead of a server display name", () => {
    const merged = mergeDevices(
      [
        device({
          id: "server-id",
          name: "开发服务器",
          deviceId: "lan-MOCKUN3Q8K3Y",
          discoverySources: ["server"],
          primarySource: "server",
          sourceLabel: "服务器",
          isLocal: false,
          serverAvailable: true,
        }),
      ],
      [],
      device({
        id: "local-id",
        name: "MOCKUN3Q8K3Y",
        deviceId: "lan-MOCKUN3Q8K3Y",
      })
    );

    expect(merged).toHaveLength(1);
    expect(merged[0]).toMatchObject({
      name: "MOCKUN3Q8K3Y",
      primarySource: "local",
      isLocal: true,
      serverAvailable: true,
    });
  });
});
