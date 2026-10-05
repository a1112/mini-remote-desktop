import { describe, expect, it } from "vitest";
import { formatDeviceCode, normalizeDeviceCode, parseRemoteDeviceInput } from "./deviceCode";

describe("device code presentation and input", () => {
  it("formats ten digits with a leading zero as 3-3-4", () => {
    expect(formatDeviceCode("0123456789")).toBe("012 345 6789");
    expect(normalizeDeviceCode("012 345\t6789")).toBe("0123456789");
    expect(parseRemoteDeviceInput("012 345 6789")).toEqual({ deviceId: "0123456789", kind: "current" });
  });

  it("preserves old nine digit and LAN identities for existing peers", () => {
    expect(formatDeviceCode("900123456")).toBe("900 123 456");
    expect(parseRemoteDeviceInput("900 123 456")).toEqual({ deviceId: "900123456", kind: "legacy" });
    expect(parseRemoteDeviceInput("lan-LCXACE")).toEqual({ deviceId: "lan-LCXACE", kind: "legacy" });
    expect(parseRemoteDeviceInput("remote-1")).toEqual({ deviceId: "remote-1", kind: "legacy" });
  });

  it("does not silently remove punctuation or accept malformed new codes", () => {
    expect(parseRemoteDeviceInput("0123456789/password")).toBeNull();
    expect(parseRemoteDeviceInput("012345678")).toEqual({ deviceId: "012345678", kind: "legacy" });
    expect(parseRemoteDeviceInput("01234567")).toBeNull();
    expect(parseRemoteDeviceInput("01234567890")).toBeNull();
    expect(parseRemoteDeviceInput("")).toBeNull();
    expect(parseRemoteDeviceInput("１２３４５６７８９０")).toBeNull();
  });
});
