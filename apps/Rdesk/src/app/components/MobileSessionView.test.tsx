import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { MobileSessionView } from "./MobileSessionView";

it("offers the real display action while a native stream has no display window", async () => {
  const openDisplay = vi.fn();
  render(<MobileSessionView deviceName="办公室电脑" status="远程媒体传输中"
    awaitingApproval={false} streaming hasNativeDisplay={false} canOpenDisplay
    error={null} suggestedAction={null} onDisconnect={vi.fn()} onOpenDisplay={openDisplay} />);
  await userEvent.setup().click(screen.getByRole("button", { name: "打开远程画面" }));
  expect(openDisplay).toHaveBeenCalledOnce();
});
