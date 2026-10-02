import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { DeviceRegisterModal } from "./DeviceRegisterModal";

const enroll = vi.hoisted(() => vi.fn());
const recover = vi.hoisted(() => vi.fn());
vi.mock("../services/deviceService", () => ({ deviceService: { enroll, recoverDeviceCredential: recover } }));

describe("DeviceRegisterModal enrollment", () => {
  beforeEach(() => {
    enroll.mockReset();
    recover.mockReset();
    window.__TAURI__ = { invoke: vi.fn().mockResolvedValue({
      motherboard_serial: "serial", hostname: "Office PC", os_type: "windows",
      os_version: "Windows 11", cpu_info: { name: "CPU", vendor_id: "Intel", cores: 4 },
      total_memory_mb: 8192, gpu_info: [],
    }) };
  });

  it("recovers an existing device with an administrator-issued credential", async () => {
    recover.mockResolvedValue({ device_id: "123456789", device_name: "Office PC", access_token: "refreshed-token" });
    render(<DeviceRegisterModal isOpen onClose={vi.fn()} />);
    fireEvent.click(await screen.findByRole("radio", { name: "更新设备凭据" }));
    fireEvent.change(screen.getByLabelText("新设备凭据"), { target: { value: "admin.rotated.token" } });
    fireEvent.click(screen.getByRole("button", { name: "更新设备凭据" }));
    await waitFor(() => expect(recover).toHaveBeenCalledWith("admin.rotated.token"));
    expect(enroll).not.toHaveBeenCalled();
  });
  afterEach(cleanup);

  it("requires a one-time enrollment code before submitting registration", async () => {
    render(<DeviceRegisterModal isOpen onClose={vi.fn()} />);
    const register = await screen.findByRole("button", { name: "注册设备" });
    expect(register).toBeDisabled();
    expect(screen.getByLabelText("设备登记码")).toHaveAttribute("type", "password");
    expect(enroll).not.toHaveBeenCalled();
  });

  it("passes the enrollment code to the service and clears it after success", async () => {
    const onSuccess = vi.fn();
    enroll.mockResolvedValue({ device_id: "123456789", device_name: "Office PC", access_token: "device-token" });
    const { rerender } = render(<DeviceRegisterModal isOpen onClose={vi.fn()} onSuccess={onSuccess} />);
    const codeInput = await screen.findByLabelText("设备登记码");
    fireEvent.change(codeInput, { target: { value: "a".repeat(43) } });
    fireEvent.click(screen.getByRole("button", { name: "注册设备" }));

    await waitFor(() => expect(enroll).toHaveBeenCalledWith("a".repeat(43), "Office PC"));
    expect(onSuccess).toHaveBeenCalledWith("123456789", "Office PC", "device-token");
    rerender(<DeviceRegisterModal isOpen={false} onClose={vi.fn()} />);
    rerender(<DeviceRegisterModal isOpen onClose={vi.fn()} />);
    expect(await screen.findByLabelText("设备登记码")).toHaveValue("");
  });
});
