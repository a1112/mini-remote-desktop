import { beforeEach, describe, expect, it } from "vitest";
import { Monitor } from "lucide-react";

import {
  lanPeerPlatformLabel,
  removeDeviceLocally,
  setDeviceDisabled,
  setDeviceFavorite,
  type Device,
  mergeDevices,
} from "./deviceData";

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
  disabled: false,
  discoverySources: ["local"],
  primarySource: "local",
  sourceLabel: "本机",
  isLocal: true,
  p2pAvailable: false,
  serverAvailable: false,
  ...overrides,
});

describe("mergeDevices", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("infers macOS LAN peers from native media capabilities", () => {
    expect(
      lanPeerPlatformLabel({
        device_type: "rdesk",
        media_capabilities: [
          "macos_capture",
          "videotoolbox_hevc",
          "videotoolbox",
          "media.hevc_main_420_8bit",
          "macos_native_render",
        ],
      })
    ).toBe("macOS");
  });

  it("prefers P2P platform labels over stale server OS labels", () => {
    const merged = mergeDevices(
      [
        device({
          id: "server-id",
          deviceId: "peer-device",
          os: "Windows",
          discoverySources: ["server"],
          primarySource: "server",
          sourceLabel: "服务器",
          isLocal: false,
          serverAvailable: true,
        }),
      ],
      [
        device({
          id: "peer-device",
          deviceId: "peer-device",
          os: "macOS / quic_datagram",
          discoverySources: ["lan_p2p"],
          primarySource: "lan_p2p",
          sourceLabel: "P2P 局域网",
          isLocal: false,
          p2pAvailable: true,
          serverAvailable: false,
        }),
      ],
      null
    );

    expect(merged[0]).toMatchObject({
      os: "macOS / quic_datagram",
      primarySource: "lan_p2p",
      p2pAvailable: true,
      serverAvailable: true,
    });
  });

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

  it("applies local favorite state and sorts favorites before ordinary peers", () => {
    setDeviceFavorite("peer-favorite", true);

    const merged = mergeDevices(
      [],
      [
        device({
          id: "peer-normal",
          name: "Normal",
          deviceId: "peer-normal",
          isLocal: false,
          discoverySources: ["lan_p2p"],
          primarySource: "lan_p2p",
          sourceLabel: "P2P 局域网",
          p2pAvailable: true,
        }),
        device({
          id: "peer-favorite",
          name: "Favorite",
          deviceId: "peer-favorite",
          isLocal: false,
          discoverySources: ["lan_p2p"],
          primarySource: "lan_p2p",
          sourceLabel: "P2P 局域网",
          p2pAvailable: true,
        }),
      ],
      null
    );

    expect(merged.map((item) => item.deviceId)).toEqual(["peer-favorite", "peer-normal"]);
    expect(merged[0]?.favorite).toBe(true);
  });

  it("applies local disabled state without removing the peer from the list", () => {
    setDeviceDisabled("peer-disabled", true);

    const merged = mergeDevices(
      [],
      [
        device({
          id: "peer-disabled",
          name: "Disabled",
          deviceId: "peer-disabled",
          isLocal: false,
          discoverySources: ["lan_p2p"],
          primarySource: "lan_p2p",
          sourceLabel: "P2P 局域网",
          p2pAvailable: true,
        }),
      ],
      null
    );

    expect(merged[0]).toMatchObject({
      deviceId: "peer-disabled",
      disabled: true,
      status: "offline",
      p2pAvailable: false,
      serverAvailable: false,
      lastSeen: "已禁用",
    });
  });

  it("hides locally removed peers but never hides the local device", () => {
    setDeviceDisabled("peer-removed", true);
    removeDeviceLocally("peer-removed");
    removeDeviceLocally("local-device");

    const merged = mergeDevices(
      [],
      [
        device({
          id: "peer-removed",
          name: "Removed",
          deviceId: "peer-removed",
          isLocal: false,
          discoverySources: ["lan_p2p"],
          primarySource: "lan_p2p",
          sourceLabel: "P2P 局域网",
          p2pAvailable: true,
        }),
      ],
      device({
        id: "local-device",
        name: "Local",
        deviceId: "local-device",
      })
    );

    expect(merged.map((item) => item.deviceId)).toEqual(["local-device"]);
    expect(merged[0]?.isLocal).toBe(true);
  });
});
