import type { DeviceInfo } from "./types";

export class DeviceStore {
  private readonly map = new Map<string, DeviceInfo>();
  private readonly seenAt = new Map<string, number>();
  private readonly staleMs: number;

  constructor(staleMs = 15_000) {
    this.staleMs = staleMs;
  }

  upsertMany(devices: DeviceInfo[], now = Date.now()): void {
    for (const device of devices) {
      this.map.set(device.id, { ...device, online: true });
      this.seenAt.set(device.id, now);
    }
    this.prune(now);
  }

  markOffline(deviceId: string): void {
    const hit = this.map.get(deviceId);
    if (!hit) {
      return;
    }
    this.map.set(deviceId, { ...hit, online: false });
    this.seenAt.delete(deviceId);
  }

  prune(now = Date.now()): void {
    for (const [id, last] of this.seenAt.entries()) {
      if (now - last > this.staleMs) {
        this.map.delete(id);
        this.seenAt.delete(id);
      }
    }
  }

  list(): DeviceInfo[] {
    return Array.from(this.map.values()).sort((a, b) => a.name.localeCompare(b.name));
  }
}
