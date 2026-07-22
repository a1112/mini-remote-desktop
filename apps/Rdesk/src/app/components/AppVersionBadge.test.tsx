import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { AppVersionBadge } from "./AppVersionBadge";

const expectedVersion = `v${__APP_VERSION__}`;

describe("AppVersionBadge", () => {
  it("renders the Tauri app version in the expanded sidebar footer", () => {
    render(<AppVersionBadge collapsed={false} isDark={false} />);

    expect(screen.getByText(expectedVersion)).toBeInTheDocument();
  });

  it("keeps the full version discoverable when the sidebar is collapsed", () => {
    render(<AppVersionBadge collapsed={true} isDark={true} />);

    expect(screen.getByTitle(`Rdesk ${expectedVersion}`)).toBeInTheDocument();
  });
});
