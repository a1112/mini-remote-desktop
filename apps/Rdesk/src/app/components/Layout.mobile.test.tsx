import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";
import { Layout } from "./Layout";

const mobile = vi.hoisted(() => ({ value: true }));

vi.mock("./ui/use-mobile", () => ({ useIsMobile: () => mobile.value }));
vi.mock("./MobileLayout", () => ({ MobileLayout: ({ onOpenAuth }: { onOpenAuth: () => void }) => <>
  <nav aria-label="移动端导航"><a href="/devices">设备</a><a href="/connections">记录</a></nav>
  <button onClick={onOpenAuth}>登录账户</button>
</> }));
vi.mock("./ThemeContext", () => ({ useTheme: () => ({ isDark: true }) }));
vi.mock("./Sidebar", () => ({ Sidebar: () => <div data-testid="desktop-sidebar" /> }));
vi.mock("./TitleBar", () => ({ TitleBar: () => <div data-testid="desktop-titlebar" /> }));
vi.mock("./ServiceStatusPanel", () => ({ ServiceStatusPanel: () => null }));
vi.mock("./ConnectionsModal", () => ({ ConnectionsModal: () => null }));
vi.mock("./SettingsModal", () => ({ SettingsModal: () => null }));
vi.mock("./FileTransferPage", () => ({ TransferModal: () => null }));
vi.mock("./AuthModal", () => ({ AuthModal: ({ open }: { open: boolean }) => open ? <div role="dialog">登录弹窗</div> : null }));

describe("responsive root layout", () => {
  it("shows the mobile tabs instead of desktop chrome on a phone", () => {
    mobile.value = true;
    render(<MemoryRouter><Layout /></MemoryRouter>);
    expect(screen.getByRole("navigation", { name: "移动端导航" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "设备" })).toHaveAttribute("href", "/devices");
    expect(screen.getByRole("link", { name: "记录" })).toHaveAttribute("href", "/connections");
    expect(screen.queryByTestId("desktop-sidebar")).not.toBeInTheDocument();
  });

  it("keeps the existing desktop chrome above the mobile breakpoint", () => {
    mobile.value = false;
    render(<MemoryRouter><Layout /></MemoryRouter>);
    expect(screen.getByTestId("desktop-sidebar")).toBeInTheDocument();
    expect(screen.queryByRole("navigation", { name: "移动端导航" })).not.toBeInTheDocument();
  });

  it("opens the existing login dialog from the mobile shell", async () => {
    mobile.value = true;
    const user = userEvent.setup();
    render(<MemoryRouter><Layout /></MemoryRouter>);
    await user.click(screen.getByRole("button", { name: "登录账户" }));
    expect(screen.getByRole("dialog", { name: "" })).toHaveTextContent("登录弹窗");
  });
});
