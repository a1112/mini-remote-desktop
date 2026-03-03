import { describe, expect, test } from "vitest";
import { DeviceStore } from "./deviceStore";

describe("DeviceStore", () => {
  test("upsert and prune stale devices", () => {
    const store = new DeviceStore(1000);
    store.upsertMany([{ id: "a", name: "A", online: true }], 0);
    expect(store.list()).toHaveLength(1);
    store.prune(1500);
    expect(store.list()).toHaveLength(0);
  });

  test("mark offline", () => {
    const store = new DeviceStore();
    store.upsertMany([{ id: "a", name: "A", online: true }], 0);
    store.markOffline("a");
    expect(store.list()[0]?.online).toBe(false);
  });
});
