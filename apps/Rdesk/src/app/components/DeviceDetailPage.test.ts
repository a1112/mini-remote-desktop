import { describe, expect, it } from "vitest";

import { deviceDetailTabFromSearch } from "./DeviceDetailPage";

describe("deviceDetailTabFromSearch", () => {
  it("opens supported tabs from the sidebar query string", () => {
    expect(deviceDetailTabFromSearch("?tab=files")).toBe("files");
    expect(deviceDetailTabFromSearch("?tab=apps")).toBe("apps");
    expect(deviceDetailTabFromSearch("?tab=info")).toBe("info");
  });

  it("falls back to the remote tab for unsupported values", () => {
    expect(deviceDetailTabFromSearch("")).toBe("remote");
    expect(deviceDetailTabFromSearch("?tab=terminal")).toBe("remote");
  });
});
