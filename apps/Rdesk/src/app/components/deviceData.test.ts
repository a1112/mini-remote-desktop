import { beforeEach, describe, expect, it } from "vitest";
import { Monitor } from "lucide-react";

import { lanPeerPlatformLabel, lanPeerToDevice, type Device, mergeDevices } from "./deviceData";

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
  beforeEach(() => {
    localStorage.clear();
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

  it("preserves LAN peer MAC addresses for Wake-on-LAN device actions", () => {
    const lanDevice = lanPeerToDevice({
      device_id: "peer-device",
      device_name: "Peer Device",
      device_type: "rdesk",
      ip: "192.168.1.20",
      discovery_port: 21116,
      p2p_control_addr: "192.168.1.20:21116",
      transports: ["quic_datagram"],
      protocol_version: 1,
      service_build_id: "test-build",
      media_protocol_version: 3,
      media_capabilities: [],
      mac_address: "AA:BB:CC:DD:EE:FF",
      age_ms: 1500,
      p2p_available: false,
    });

    expect(lanDevice).toMatchObject({
      deviceId: "peer-device",
      status: "offline",
      macAddress: "AA:BB:CC:DD:EE:FF",
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

  it("applies local device action preferences to merged devices", () => {
    localStorage.setItem(
      "rdesk_device_action_preferences",
      JSON.stringify({
        "favorite-device": { favorite: true },
        "disabled-device": { disabled: true },
        "removed-device": { removed: true },
      })
    );

    const merged = mergeDevices(
      [
        device({
          id: "server-favorite",
          name: "Favorite Peer",
          deviceId: "favorite-device",
          favorite: false,
          discoverySources: ["server"],
          primarySource: "server",
          sourceLabel: "服务器",
          isLocal: false,
          serverAvailable: true,
        }),
        device({
          id: "server-removed",
          name: "Removed Peer",
          deviceId: "removed-device",
          discoverySources: ["server"],
          primarySource: "server",
          sourceLabel: "服务器",
          isLocal: false,
          serverAvailable: true,
        }),
        device({
          id: "server-disabled",
          name: "Disabled Peer",
          deviceId: "disabled-device",
          discoverySources: ["server"],
          primarySource: "server",
          sourceLabel: "服务器",
          isLocal: false,
          serverAvailable: true,
        }),
      ],
      [],
      null
    );

    expect(merged.map((item) => item.deviceId)).toEqual([
      "favorite-device",
      "disabled-device",
    ]);
    expect(merged.find((item) => item.deviceId === "favorite-device")).toMatchObject({
      favorite: true,
    });
    expect(merged.find((item) => item.deviceId === "disabled-device")).toMatchObject({
      disabled: true,
      status: "offline",
    });
  });
});
