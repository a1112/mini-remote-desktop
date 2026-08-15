import { useCallback, useEffect, useRef, useState } from "react";

import {
  ipcGetRemoteSession,
  ipcRespondToConsent,
  ipcSubscribeSessionEvents,
  showWindow,
} from "../adapters/tauri";
import type {
  ConsentResponse,
  RemoteSessionSnapshot,
} from "../adapters/tauri/types";
import { IncomingSessionDialog } from "./IncomingSessionDialog";
import { getTauriWindowLabel } from "../utils/tauriWindow";

const EVENT_PAGE_SIZE = 64;
const EVENT_WAIT_TIMEOUT_MS = 15_000;
const RETRY_AFTER_ERROR_MS = 250;
const FALLBACK_CONSENT_LIFETIME_MS = 30_000;

function isPendingAttendedAgent(session: RemoteSessionSnapshot) {
  return (
    session.role === "agent" &&
    session.access_mode === "attended" &&
    session.authorization_state === "awaiting_local_consent"
  );
}

export function IncomingSessionConsentHost() {
  const [pendingSessions, setPendingSessions] = useState<
    RemoteSessionSnapshot[]
  >([]);
  const seenSessionIds = useRef(new Set<string>());

  useEffect(() => {
    let stopped = false;
    let timer: number | undefined;
    let cursor: string | null = null;

    const schedule = (delayMs: number) => {
      if (stopped) {
        return;
      }
      timer = window.setTimeout(() => {
        void poll();
      }, delayMs);
    };

    const enqueuePendingSession = (snapshot: RemoteSessionSnapshot) => {
      if (
        !isPendingAttendedAgent(snapshot) ||
        seenSessionIds.current.has(snapshot.session_id)
      ) {
        return;
      }
      seenSessionIds.current.add(snapshot.session_id);
      setPendingSessions((current) => [...current, snapshot]);
    };

    const poll = async (): Promise<void> => {
      let result: Awaited<ReturnType<typeof ipcSubscribeSessionEvents>>;
      try {
        result = await ipcSubscribeSessionEvents({
          session_id: null,
          after_sequence: cursor,
          limit: EVENT_PAGE_SIZE,
          wait_timeout_ms: EVENT_WAIT_TIMEOUT_MS,
        });
      } catch {
        schedule(RETRY_AFTER_ERROR_MS);
        return;
      }
      if (stopped) {
        return;
      }
      if (!result.ok) {
        schedule(RETRY_AFTER_ERROR_MS);
        return;
      }

      const page = result.value;
      if (page.cursor_state === "reset_required") {
        const authoritativePending = (page.pending_sessions ?? []).filter(
          isPendingAttendedAgent,
        );
        seenSessionIds.current = new Set(
          authoritativePending.map((snapshot) => snapshot.session_id),
        );
        setPendingSessions(authoritativePending);
        cursor = page.next_after_sequence ?? cursor;
        schedule(page.poll_after_ms);
        return;
      }
      for (const snapshot of page.pending_sessions ?? []) {
        enqueuePendingSession(snapshot);
      }

      let snapshotLookupFailed = false;
      for (const envelope of page.events) {
        if (stopped) {
          continue;
        }
        if (
          envelope.event.kind === "consent_resolved" ||
          envelope.event.kind === "authorization_changed" ||
          envelope.event.kind === "session_closed"
        ) {
          setPendingSessions((current) =>
            current.filter(
              (session) => session.session_id !== envelope.session_id,
            ),
          );
          seenSessionIds.current.delete(envelope.session_id);
          continue;
        }
        if (envelope.event.kind !== "consent_requested") {
          continue;
        }
        if (seenSessionIds.current.has(envelope.session_id)) {
          continue;
        }
        let snapshotResult: Awaited<ReturnType<typeof ipcGetRemoteSession>>;
        try {
          snapshotResult = await ipcGetRemoteSession(envelope.session_id);
        } catch {
          snapshotLookupFailed = true;
          continue;
        }
        if (stopped) {
          continue;
        }
        if (!snapshotResult.ok) {
          if (snapshotResult.error.code !== "E_REMOTE_SESSION_NOT_FOUND") {
            snapshotLookupFailed = true;
          }
          continue;
        }
        enqueuePendingSession(snapshotResult.value);
      }

      if (snapshotLookupFailed) {
        schedule(RETRY_AFTER_ERROR_MS);
        return;
      }
      cursor = page.next_after_sequence ?? cursor;
      schedule(page.has_more ? 0 : page.poll_after_ms);
    };

    void (async () => {
      const label = await getTauriWindowLabel();
      if (stopped || (label !== null && label !== "main")) {
        return;
      }
      await poll();
    })();
    return () => {
      stopped = true;
      if (timer !== undefined) {
        window.clearTimeout(timer);
      }
    };
  }, []);

  const respond = useCallback(async (response: ConsentResponse) => {
    const result = await ipcRespondToConsent(response);
    if (!result.ok) {
      throw new Error(result.error.message);
    }
    setPendingSessions((current) =>
      current.filter((session) => session.session_id !== response.session_id),
    );
    seenSessionIds.current.delete(response.session_id);
  }, []);

  const current = pendingSessions[0] ?? null;
  const consentDeadlineMs = current
    ? (current.authorization_expires_at_ms ??
      current.created_at_ms + FALLBACK_CONSENT_LIFETIME_MS)
    : 0;

  useEffect(() => {
    if (!current) {
      return;
    }

    let cancelled = false;
    let retryTimer: number | undefined;

    const reveal = async (): Promise<void> => {
      if (cancelled || Date.now() >= consentDeadlineMs) {
        return;
      }

      let shown = false;
      try {
        shown = (await showWindow()).ok;
      } catch {
        shown = false;
      }
      if (cancelled || shown || Date.now() >= consentDeadlineMs) {
        return;
      }

      retryTimer = window.setTimeout(() => {
        retryTimer = undefined;
        void reveal();
      }, RETRY_AFTER_ERROR_MS);
    };

    void reveal();
    return () => {
      cancelled = true;
      if (retryTimer !== undefined) {
        window.clearTimeout(retryTimer);
      }
    };
  }, [consentDeadlineMs, current?.session_id]);

  return (
    <IncomingSessionDialog
      consentDeadlineMs={consentDeadlineMs}
      onRespond={respond}
      session={current}
    />
  );
}
