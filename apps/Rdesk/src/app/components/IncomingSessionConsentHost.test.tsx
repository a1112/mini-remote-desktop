import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  RemoteSessionSnapshot,
  SessionEventSubscription,
} from "../adapters/tauri/types";

const adapter = vi.hoisted(() => ({
  getRemoteSession: vi.fn(),
  respondToConsent: vi.fn(),
  showWindow: vi.fn(),
  subscribeSessionEvents: vi.fn(),
}));
const windowApi = vi.hoisted(() => ({
  getLabel: vi.fn(),
}));

vi.mock("../adapters/tauri", () => ({
  ipcGetRemoteSession: adapter.getRemoteSession,
  ipcRespondToConsent: adapter.respondToConsent,
  ipcSubscribeSessionEvents: adapter.subscribeSessionEvents,
  showWindow: adapter.showWindow,
}));

vi.mock("../utils/tauriWindow", () => ({
  getTauriWindowLabel: windowApi.getLabel,
}));

vi.mock("../routes", async () => {
  const React = await import("react");
  const { createMemoryRouter } = await import("react-router");
  return {
    router: createMemoryRouter([
      {
        path: "*",
        element: React.createElement("div", null, "application route"),
      },
    ]),
  };
});

import App from "../App";
import { IncomingSessionConsentHost } from "./IncomingSessionConsentHost";

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
    requested_scopes: ["screen.view", "input.pointer"],
    granted_scopes: [],
    policy_revision: "7",
    failure: null,
    created_at_ms: Date.now(),
    updated_at_ms: Date.now(),
    ...overrides,
  };
}

function subscription(
  overrides: Partial<SessionEventSubscription> = {},
): SessionEventSubscription {
  return {
    events: [
      {
        sequence: "9007199254740993",
        timestamp_ms: Date.now(),
        session_id: "session-1",
        event: {
          kind: "consent_requested",
          requested_scopes: ["screen.view", "input.pointer"],
        },
      },
    ],
    pending_sessions: [],
    next_after_sequence: "9007199254740993",
    cursor_state: "current",
    has_more: false,
    poll_after_ms: 60_000,
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

describe("IncomingSessionConsentHost", () => {
  beforeEach(() => {
    adapter.getRemoteSession.mockReset();
    adapter.respondToConsent.mockReset();
    adapter.showWindow.mockReset();
    adapter.subscribeSessionEvents.mockReset();
    windowApi.getLabel.mockReset();
    windowApi.getLabel.mockResolvedValue("main");
    adapter.showWindow.mockResolvedValue({ ok: true, value: undefined });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("fetches the authoritative snapshot for a consent event and shows it", async () => {
    adapter.subscribeSessionEvents.mockResolvedValueOnce({
      ok: true,
      value: subscription(),
    });
    adapter.getRemoteSession.mockResolvedValueOnce({
      ok: true,
      value: makeSession(),
    });

    render(<IncomingSessionConsentHost />);

    expect(
      await screen.findByRole("heading", { name: "Incoming remote session" }),
    ).toBeInTheDocument();
    expect(screen.getByText("device-living-room")).toBeInTheDocument();
    expect(adapter.getRemoteSession).toHaveBeenCalledWith("session-1");
    expect(adapter.showWindow).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(1);
    });
  });

  it("closes a queued request when authorization changes before a response", async () => {
    let resolveSecondPoll!: (value: {
      ok: true;
      value: SessionEventSubscription;
    }) => void;
    const secondPoll = new Promise<{
      ok: true;
      value: SessionEventSubscription;
    }>((resolve) => {
      resolveSecondPoll = resolve;
    });
    adapter.subscribeSessionEvents
      .mockResolvedValueOnce({
        ok: true,
        value: subscription({ poll_after_ms: 0 }),
      })
      .mockReturnValueOnce(secondPoll)
      .mockImplementation(() => new Promise(() => undefined));
    adapter.getRemoteSession.mockResolvedValueOnce({
      ok: true,
      value: makeSession(),
    });

    render(<IncomingSessionConsentHost />);

    expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
    await waitFor(() => {
      expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(2);
    });

    resolveSecondPoll({
      ok: true,
      value: subscription({
        events: [
          {
            sequence: "9007199254740994",
            timestamp_ms: Date.now(),
            session_id: "session-1",
            event: {
              kind: "authorization_changed",
              state: "expired",
              failure: {
                code: "authorization_timeout",
                message: "request expired",
              },
            },
          },
        ],
        next_after_sequence: "9007199254740994",
        poll_after_ms: 60_000,
      }),
    });

    await waitFor(() => {
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    });
  });

  it("rehydrates pending consent from a reset page before advancing its cursor", async () => {
    const firstPoll = deferred<{
      ok: true;
      value: SessionEventSubscription;
    }>();
    const secondPoll = deferred<{
      ok: true;
      value: SessionEventSubscription;
    }>();
    adapter.subscribeSessionEvents
      .mockReturnValueOnce(firstPoll.promise)
      .mockReturnValueOnce(secondPoll.promise);

    render(<IncomingSessionConsentHost />);

    await waitFor(() => {
      expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(1);
    });
    await Promise.resolve();
    expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(1);

    firstPoll.resolve({
      ok: true,
      value: subscription({
        events: [],
        pending_sessions: [makeSession()],
        next_after_sequence: "18446744073709551614",
        cursor_state: "reset_required",
        poll_after_ms: 0,
      }),
    });

    await waitFor(() => {
      expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(2);
    });
    expect(
      await screen.findByRole("heading", { name: "Incoming remote session" }),
    ).toBeInTheDocument();
    expect(adapter.getRemoteSession).not.toHaveBeenCalled();
    expect(adapter.subscribeSessionEvents).toHaveBeenLastCalledWith({
      session_id: null,
      after_sequence: "18446744073709551614",
      limit: 64,
      wait_timeout_ms: 15_000,
    });
  });

  it("reconciles stale local consent state from an authoritative reset page", async () => {
    adapter.subscribeSessionEvents
      .mockResolvedValueOnce({
        ok: true,
        value: subscription({ poll_after_ms: 0 }),
      })
      .mockResolvedValueOnce({
        ok: true,
        value: subscription({
          events: [],
          pending_sessions: [
            makeSession({
              session_id: "session-2",
              peer_device_id: "device-office",
            }),
          ],
          next_after_sequence: "99",
          cursor_state: "reset_required",
          poll_after_ms: 60_000,
        }),
      })
      .mockImplementation(() => new Promise(() => undefined));
    adapter.getRemoteSession.mockResolvedValueOnce({
      ok: true,
      value: makeSession(),
    });

    render(<IncomingSessionConsentHost />);

    expect(await screen.findByText("device-office")).toBeInTheDocument();
    expect(screen.queryByText("device-living-room")).not.toBeInTheDocument();
  });

  it("keeps retrying the main window until a pending consent can be shown", async () => {
    vi.useFakeTimers();
    adapter.subscribeSessionEvents
      .mockResolvedValueOnce({ ok: true, value: subscription() })
      .mockImplementation(() => new Promise(() => undefined));
    adapter.getRemoteSession.mockResolvedValueOnce({
      ok: true,
      value: makeSession(),
    });
    adapter.showWindow
      .mockResolvedValueOnce({
        ok: false,
        error: { code: "E_WINDOW", message: "window unavailable" },
      })
      .mockResolvedValueOnce({
        ok: false,
        error: { code: "E_WINDOW", message: "window unavailable" },
      })
      .mockResolvedValueOnce({
        ok: false,
        error: { code: "E_WINDOW", message: "window unavailable" },
      })
      .mockResolvedValueOnce({
        ok: false,
        error: { code: "E_WINDOW", message: "window unavailable" },
      })
      .mockResolvedValueOnce({
        ok: false,
        error: { code: "E_WINDOW", message: "window unavailable" },
      })
      .mockResolvedValueOnce({ ok: true, value: undefined });

    render(<IncomingSessionConsentHost />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(adapter.showWindow).toHaveBeenCalledTimes(1);

    for (let attempt = 2; attempt <= 6; attempt += 1) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(250);
      });
      expect(adapter.showWindow).toHaveBeenCalledTimes(attempt);
    }

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(adapter.showWindow).toHaveBeenCalledTimes(6);
  });

  it("cancels a queued window retry when the pending consent is resolved", async () => {
    vi.useFakeTimers();
    const secondPoll = deferred<{
      ok: true;
      value: SessionEventSubscription;
    }>();
    adapter.subscribeSessionEvents
      .mockResolvedValueOnce({
        ok: true,
        value: subscription({ poll_after_ms: 0 }),
      })
      .mockReturnValueOnce(secondPoll.promise)
      .mockImplementation(() => new Promise(() => undefined));
    adapter.getRemoteSession.mockResolvedValueOnce({
      ok: true,
      value: makeSession(),
    });
    adapter.showWindow.mockResolvedValue({
      ok: false,
      error: { code: "E_WINDOW", message: "window unavailable" },
    });

    render(<IncomingSessionConsentHost />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(adapter.showWindow).toHaveBeenCalledTimes(1);
    expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(2);

    secondPoll.resolve({
      ok: true,
      value: subscription({
        events: [
          {
            sequence: "9007199254740994",
            timestamp_ms: Date.now(),
            session_id: "session-1",
            event: {
              kind: "authorization_changed",
              state: "denied",
              failure: {
                code: "consent_denied",
                message: "request denied elsewhere",
              },
            },
          },
        ],
        next_after_sequence: "9007199254740994",
        poll_after_ms: 60_000,
      }),
    });
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(adapter.showWindow).toHaveBeenCalledTimes(1);
  });

  it("retries after an unexpected subscription rejection", async () => {
    vi.useFakeTimers();
    adapter.subscribeSessionEvents
      .mockRejectedValueOnce(new Error("bridge unavailable"))
      .mockImplementation(() => new Promise(() => undefined));

    render(<IncomingSessionConsentHost />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(250);
    });
    expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(2);
  });

  it("presents deduplicated consent requests in FIFO order", async () => {
    const user = userEvent.setup();
    adapter.subscribeSessionEvents
      .mockResolvedValueOnce({
        ok: true,
        value: subscription({
          events: [
            {
              sequence: "1",
              timestamp_ms: Date.now(),
              session_id: "session-1",
              event: {
                kind: "consent_requested",
                requested_scopes: ["screen.view"],
              },
            },
            {
              sequence: "2",
              timestamp_ms: Date.now(),
              session_id: "session-1",
              event: {
                kind: "consent_requested",
                requested_scopes: ["screen.view"],
              },
            },
            {
              sequence: "3",
              timestamp_ms: Date.now(),
              session_id: "session-2",
              event: {
                kind: "consent_requested",
                requested_scopes: ["screen.view"],
              },
            },
          ],
          next_after_sequence: "3",
        }),
      })
      .mockImplementation(() => new Promise(() => undefined));
    adapter.getRemoteSession
      .mockResolvedValueOnce({ ok: true, value: makeSession() })
      .mockResolvedValueOnce({
        ok: true,
        value: makeSession({
          session_id: "session-2",
          peer_device_id: "device-office",
          peer_key_id: "key-ed25519:d4e5f6",
          policy_revision: "9",
        }),
      });
    adapter.respondToConsent.mockResolvedValue({
      ok: true,
      value: makeSession({ authorization_state: "denied" }),
    });

    render(<IncomingSessionConsentHost />);

    expect(await screen.findByText("device-living-room")).toBeInTheDocument();
    expect(screen.queryByText("device-office")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Deny" }));

    expect(adapter.respondToConsent).toHaveBeenCalledWith({
      session_id: "session-1",
      decision: "deny",
      approved_scopes: [],
      expected_policy_revision: "7",
    });
    expect(await screen.findByText("device-office")).toBeInTheDocument();
    expect(adapter.getRemoteSession).toHaveBeenCalledTimes(2);
  });

  it("retries the same cursor after 250ms when an authoritative snapshot lookup rejects", async () => {
    vi.useFakeTimers();
    adapter.subscribeSessionEvents
      .mockResolvedValueOnce({
        ok: true,
        value: subscription({
          events: [
            {
              sequence: "1",
              timestamp_ms: Date.now(),
              session_id: "session-1",
              event: {
                kind: "consent_requested",
                requested_scopes: ["screen.view"],
              },
            },
          ],
          next_after_sequence: "1",
          poll_after_ms: 60_000,
        }),
      })
      .mockResolvedValueOnce({
        ok: true,
        value: subscription({
          events: [
            {
              sequence: "1",
              timestamp_ms: Date.now(),
              session_id: "session-1",
              event: {
                kind: "consent_requested",
                requested_scopes: ["screen.view"],
              },
            },
          ],
          next_after_sequence: "1",
          poll_after_ms: 60_000,
        }),
      })
      .mockImplementation(() => new Promise(() => undefined));
    adapter.getRemoteSession
      .mockRejectedValueOnce(new Error("snapshot bridge unavailable"))
      .mockResolvedValueOnce({
        ok: true,
        value: makeSession(),
      });

    render(<IncomingSessionConsentHost />);

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(1);
    expect(adapter.getRemoteSession).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(249);
    });
    expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });

    expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(2);
    expect(adapter.subscribeSessionEvents).toHaveBeenNthCalledWith(2, {
      session_id: null,
      after_sequence: null,
      limit: 64,
      wait_timeout_ms: 15_000,
    });
    expect(adapter.getRemoteSession).toHaveBeenCalledTimes(2);
    expect(screen.getByText("device-living-room")).toBeInTheDocument();
  });

  it("consumes a consent event whose authoritative session was already removed", async () => {
    adapter.subscribeSessionEvents
      .mockResolvedValueOnce({
        ok: true,
        value: subscription({ poll_after_ms: 0 }),
      })
      .mockImplementation(() => new Promise(() => undefined));
    adapter.getRemoteSession.mockResolvedValueOnce({
      ok: false,
      error: {
        code: "E_REMOTE_SESSION_NOT_FOUND",
        message: "remote session not found",
      },
    });

    render(<IncomingSessionConsentHost />);

    await waitFor(() => {
      expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(2);
    });
    expect(adapter.subscribeSessionEvents).toHaveBeenLastCalledWith({
      session_id: null,
      after_sequence: "9007199254740993",
      limit: 64,
      wait_timeout_ms: 15_000,
    });
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("keeps the current request visible when consent IPC fails", async () => {
    const user = userEvent.setup();
    adapter.subscribeSessionEvents
      .mockResolvedValueOnce({ ok: true, value: subscription() })
      .mockImplementation(() => new Promise(() => undefined));
    adapter.getRemoteSession.mockResolvedValueOnce({
      ok: true,
      value: makeSession(),
    });
    adapter.respondToConsent.mockResolvedValueOnce({
      ok: false,
      error: { code: "E_CONFLICT", message: "policy changed" },
    });

    render(<IncomingSessionConsentHost />);

    const deny = await screen.findByRole("button", { name: "Deny" });
    await user.click(deny);
    await waitFor(() => {
      expect(deny).toBeEnabled();
    });
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
    expect(adapter.respondToConsent).toHaveBeenCalledTimes(1);
  });

  it("ignores controller, unattended, and non-pending authoritative snapshots", async () => {
    adapter.subscribeSessionEvents
      .mockResolvedValueOnce({
        ok: true,
        value: subscription({
          events: [
            {
              sequence: "1",
              timestamp_ms: Date.now(),
              session_id: "controller",
              event: { kind: "consent_requested", requested_scopes: [] },
            },
            {
              sequence: "2",
              timestamp_ms: Date.now(),
              session_id: "unattended",
              event: { kind: "consent_requested", requested_scopes: [] },
            },
            {
              sequence: "3",
              timestamp_ms: Date.now(),
              session_id: "already-resolved",
              event: { kind: "consent_requested", requested_scopes: [] },
            },
          ],
        }),
      })
      .mockImplementation(() => new Promise(() => undefined));
    adapter.getRemoteSession
      .mockResolvedValueOnce({
        ok: true,
        value: makeSession({ role: "controller" }),
      })
      .mockResolvedValueOnce({
        ok: true,
        value: makeSession({ access_mode: "unattended" }),
      })
      .mockResolvedValueOnce({
        ok: true,
        value: makeSession({ authorization_state: "authorizing" }),
      });

    render(<IncomingSessionConsentHost />);

    await waitFor(() => {
      expect(adapter.getRemoteSession).toHaveBeenCalledTimes(3);
    });
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("uses the authoritative authorization expiry when the service provides it", async () => {
    const now = Date.now();
    adapter.subscribeSessionEvents
      .mockResolvedValueOnce({ ok: true, value: subscription() })
      .mockImplementation(() => new Promise(() => undefined));
    adapter.getRemoteSession.mockResolvedValueOnce({
      ok: true,
      value: makeSession({
        created_at_ms: now,
        authorization_expires_at_ms: now - 1,
      }),
    });

    render(<IncomingSessionConsentHost />);

    expect(await screen.findByText("This request has expired.")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Allow selected permissions" }),
    ).toBeDisabled();
  });

  it("stops safely when an in-flight poll resolves after teardown", async () => {
    const poll = deferred<{ ok: true; value: SessionEventSubscription }>();
    adapter.subscribeSessionEvents.mockReturnValueOnce(poll.promise);
    const view = render(<IncomingSessionConsentHost />);
    await waitFor(() => {
      expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(1);
    });

    view.unmount();
    poll.resolve({ ok: true, value: subscription({ poll_after_ms: 0 }) });
    await Promise.resolve();
    await Promise.resolve();

    expect(adapter.getRemoteSession).not.toHaveBeenCalled();
    expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(1);
  });

  it("mounts the consent subscriber globally at the application root", async () => {
    adapter.subscribeSessionEvents.mockImplementation(
      () => new Promise(() => undefined),
    );

    render(<App />);

    expect(screen.getByText("application route")).toBeInTheDocument();
    await waitFor(() => {
      expect(adapter.subscribeSessionEvents).toHaveBeenCalledTimes(1);
    });
  });

  it("does not subscribe from a controller display window", async () => {
    windowApi.getLabel.mockResolvedValue("remote-display-session-1");

    render(<IncomingSessionConsentHost />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(adapter.subscribeSessionEvents).not.toHaveBeenCalled();
  });
});
