import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { RemoteSessionSnapshot } from "../adapters/tauri/types";
import { IncomingSessionDialog } from "./IncomingSessionDialog";

function makeSession(
  overrides: Partial<RemoteSessionSnapshot> = {},
): RemoteSessionSnapshot {
  return {
    session_id: "session-1",
    role: "agent",
    peer_device_id: "device-living-room",
    peer_key_id: "key-ed25519:a1b2c3",
    access_mode: "attended",
    authorization_state: "awaiting_local_consent",
    route_state: "idle",
    route_kind: "lan_quic",
    media_state: "idle",
    presentation_state: "incoming_approval_required",
    requested_scopes: ["screen.view", "input.pointer", "clipboard.read"],
    granted_scopes: [],
    policy_revision: "7",
    failure: null,
    created_at_ms: 1_700_000_000_000,
    updated_at_ms: 1_700_000_000_000,
    ...overrides,
  };
}

describe("IncomingSessionDialog", () => {
  it("shows the exact peer identity and requested scope names", () => {
    render(
      <IncomingSessionDialog
        session={makeSession()}
        consentDeadlineMs={Date.now() + 30_000}
        onRespond={vi.fn()}
      />,
    );

    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
    expect(screen.getByText("device-living-room")).toBeInTheDocument();
    expect(screen.getByText("key-ed25519:a1b2c3")).toBeInTheDocument();
    expect(screen.getByText("screen.view")).toBeInTheDocument();
    expect(screen.getByText("input.pointer")).toBeInTheDocument();
    expect(screen.getByText("clipboard.read")).toBeInTheDocument();
  });

  it("defaults to only the screen viewing permission", () => {
    render(
      <IncomingSessionDialog
        session={makeSession()}
        consentDeadlineMs={Date.now() + 30_000}
        onRespond={vi.fn()}
      />,
    );

    expect(screen.getByRole("checkbox", { name: "screen.view" })).toBeChecked();
    expect(
      screen.getByRole("checkbox", { name: "input.pointer" }),
    ).not.toBeChecked();
    expect(
      screen.getByRole("checkbox", { name: "clipboard.read" }),
    ).not.toBeChecked();
  });

  it("submits only the explicitly selected permission subset", async () => {
    const user = userEvent.setup();
    const onRespond = vi.fn().mockResolvedValue(undefined);
    render(
      <IncomingSessionDialog
        session={makeSession()}
        consentDeadlineMs={Date.now() + 30_000}
        onRespond={onRespond}
      />,
    );

    await user.click(
      screen.getByRole("checkbox", { name: "input.pointer" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Allow selected permissions" }),
    );

    expect(onRespond).toHaveBeenCalledWith({
      session_id: "session-1",
      decision: "approve",
      approved_scopes: ["screen.view", "input.pointer"],
      expected_policy_revision: "7",
    });
  });

  it("submits an empty permission set when denied", async () => {
    const user = userEvent.setup();
    const onRespond = vi.fn().mockResolvedValue(undefined);
    render(
      <IncomingSessionDialog
        session={makeSession()}
        consentDeadlineMs={Date.now() + 30_000}
        onRespond={onRespond}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Deny" }));

    expect(onRespond).toHaveBeenCalledWith({
      session_id: "session-1",
      decision: "deny",
      approved_scopes: [],
      expected_policy_revision: "7",
    });
  });

  it("requires an explicit selection when screen viewing was not requested", async () => {
    const user = userEvent.setup();
    render(
      <IncomingSessionDialog
        session={makeSession({ requested_scopes: ["input.pointer"] })}
        consentDeadlineMs={Date.now() + 30_000}
        onRespond={vi.fn()}
      />,
    );

    const approve = screen.getByRole("button", {
      name: "Allow selected permissions",
    });
    expect(approve).toBeDisabled();

    await user.click(
      screen.getByRole("checkbox", { name: "input.pointer" }),
    );
    expect(approve).toBeEnabled();
  });

  it("latches the first decision while its submission is pending", async () => {
    let resolveSubmission!: () => void;
    const submission = new Promise<void>((resolve) => {
      resolveSubmission = resolve;
    });
    const onRespond = vi.fn().mockReturnValue(submission);
    render(
      <IncomingSessionDialog
        session={makeSession()}
        consentDeadlineMs={Date.now() + 30_000}
        onRespond={onRespond}
      />,
    );

    const approve = screen.getByRole("button", {
      name: "Allow selected permissions",
    });
    const deny = screen.getByRole("button", { name: "Deny" });
    act(() => {
      fireEvent.click(approve);
      fireEvent.click(deny);
    });

    expect(onRespond).toHaveBeenCalledTimes(1);
    expect(approve).toBeDisabled();
    expect(deny).toBeDisabled();
    expect(
      screen.getByRole("checkbox", { name: "screen.view" }),
    ).toBeDisabled();

    await act(async () => {
      resolveSubmission();
      await submission;
    });
  });

  it("disables approval after the consent deadline expires", () => {
    render(
      <IncomingSessionDialog
        session={makeSession()}
        consentDeadlineMs={Date.now() - 1}
        onRespond={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("button", { name: "Allow selected permissions" }),
    ).toBeDisabled();
    expect(screen.getByRole("button", { name: "Deny" })).toBeEnabled();
    expect(screen.getByText("This request has expired.")).toBeInTheDocument();
  });

  it("expires an open request when its deadline is reached", () => {
    vi.useFakeTimers();
    try {
      const now = new Date("2026-07-11T08:00:00.000Z");
      vi.setSystemTime(now);
      render(
        <IncomingSessionDialog
          session={makeSession()}
          consentDeadlineMs={now.getTime() + 1_000}
          onRespond={vi.fn()}
        />,
      );

      const approve = screen.getByRole("button", {
        name: "Allow selected permissions",
      });
      expect(approve).toBeEnabled();

      act(() => {
        vi.advanceTimersByTime(1_001);
      });

      expect(approve).toBeDisabled();
      expect(screen.getByText("This request has expired.")).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("resets permission selections when the session changes", async () => {
    const user = userEvent.setup();
    const deadline = Date.now() + 30_000;
    const { rerender } = render(
      <IncomingSessionDialog
        session={makeSession()}
        consentDeadlineMs={deadline}
        onRespond={vi.fn()}
      />,
    );

    const pointerPermission = screen.getByRole("checkbox", {
      name: "input.pointer",
    });
    await user.click(pointerPermission);
    expect(pointerPermission).toBeChecked();

    rerender(
      <IncomingSessionDialog
        session={makeSession({
          session_id: "session-2",
          peer_device_id: "device-office",
          peer_key_id: "key-ed25519:d4e5f6",
        })}
        consentDeadlineMs={deadline}
        onRespond={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("checkbox", { name: "screen.view" }),
    ).toBeChecked();
    expect(
      screen.getByRole("checkbox", { name: "input.pointer" }),
    ).not.toBeChecked();
  });

  it("shows a consent submission failure and allows the user to retry", async () => {
    const user = userEvent.setup();
    const onRespond = vi
      .fn()
      .mockRejectedValueOnce(new Error("policy revision changed"))
      .mockResolvedValueOnce(undefined);
    render(
      <IncomingSessionDialog
        session={makeSession()}
        consentDeadlineMs={Date.now() + 30_000}
        onRespond={onRespond}
      />,
    );

    const deny = screen.getByRole("button", { name: "Deny" });
    await user.click(deny);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "policy revision changed",
    );
    expect(deny).toBeEnabled();

    await user.click(deny);
    await waitFor(() => expect(onRespond).toHaveBeenCalledTimes(2));
  });

  it("puts initial focus on the safe deny action", async () => {
    render(
      <IncomingSessionDialog
        session={makeSession()}
        consentDeadlineMs={Date.now() + 30_000}
        onRespond={vi.fn()}
      />,
    );

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Deny" })).toHaveFocus();
    });
  });

  it("does not silently dismiss on Escape or an overlay click", async () => {
    const user = userEvent.setup();
    const onRespond = vi.fn();
    render(
      <IncomingSessionDialog
        session={makeSession()}
        consentDeadlineMs={Date.now() + 30_000}
        onRespond={onRespond}
      />,
    );

    await user.keyboard("{Escape}");
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();

    const overlay = document.querySelector<HTMLElement>(
      '[data-slot="alert-dialog-overlay"]',
    );
    expect(overlay).not.toBeNull();
    fireEvent.pointerDown(overlay!);
    fireEvent.click(overlay!);

    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
    expect(onRespond).not.toHaveBeenCalled();
  });
});
