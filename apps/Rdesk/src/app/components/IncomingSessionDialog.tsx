import { useEffect, useId, useRef, useState } from "react";

import type {
  ConsentResponse,
  RemotePermissionScope,
  RemoteSessionSnapshot,
  PairPendingSessionRequest,
} from "../adapters/tauri/types";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "./ui/alert-dialog";
import { Button } from "./ui/button";

export interface IncomingSessionDialogProps {
  session: RemoteSessionSnapshot | null;
  consentDeadlineMs: number;
  onRespond: (response: ConsentResponse) => Promise<void>;
  onPair?: (request: PairPendingSessionRequest) => Promise<void>;
}

function defaultSelectedScopes(session: RemoteSessionSnapshot | null) {
  return session?.requested_scopes.includes("screen.view")
    ? new Set<RemotePermissionScope>(["screen.view"])
    : new Set<RemotePermissionScope>();
}

export function IncomingSessionDialog({
  session,
  consentDeadlineMs,
  onRespond,
  onPair,
}: IncomingSessionDialogProps) {
  const [selectedScopes, setSelectedScopes] = useState<
    Set<RemotePermissionScope>
  >(() => defaultSelectedScopes(session));
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [submissionError, setSubmissionError] = useState<string | null>(null);
  const submittingRef = useRef(false);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const denyButtonId = useId();

  useEffect(() => {
    setSelectedScopes(defaultSelectedScopes(session));
    submittingRef.current = false;
    setIsSubmitting(false);
    setSubmissionError(null);
  }, [session?.session_id]);

  useEffect(() => {
    const now = Date.now();
    setNowMs(now);
    const remainingMs = consentDeadlineMs - now;
    if (remainingMs <= 0) {
      return undefined;
    }

    const timeout = window.setTimeout(() => {
      setNowMs(Date.now());
    }, remainingMs);
    return () => window.clearTimeout(timeout);
  }, [consentDeadlineMs, session?.session_id]);

  const isExpired = nowMs >= consentDeadlineMs;

  if (session?.authorization_state !== "awaiting_local_consent") {
    return null;
  }

  const canPair = Boolean(
    onPair &&
      session.role === "agent" &&
      session.access_mode === "attended" &&
      session.route_kind !== "lan_quic" &&
      session.route_kind !== "webrtc_direct",
  );
  const hasScreenViewing = selectedScopes.has("screen.view");

  const toggleScope = (scope: RemotePermissionScope) => {
    setSelectedScopes((current) => {
      const next = new Set(current);
      if (next.has(scope)) {
        next.delete(scope);
      } else {
        next.add(scope);
      }
      return next;
    });
  };

  const submit = (operation: () => Promise<void>) => {
    if (submittingRef.current) {
      return;
    }

    submittingRef.current = true;
    setIsSubmitting(true);
    setSubmissionError(null);
    let submission: Promise<void>;
    try {
      submission = operation();
    } catch (error) {
      setSubmissionError(
        error instanceof Error ? error.message : "Consent response failed.",
      );
      submittingRef.current = false;
      setIsSubmitting(false);
      return;
    }

    void submission
      .catch((error: unknown) => {
        setSubmissionError(
          error instanceof Error ? error.message : "Consent response failed.",
        );
      })
      .finally(() => {
        submittingRef.current = false;
        setIsSubmitting(false);
      });
  };

  const approve = () => {
    submit(() => onRespond({
      session_id: session.session_id,
      decision: "approve",
      approved_scopes: session.requested_scopes.filter((scope) =>
        selectedScopes.has(scope),
      ),
      expected_policy_revision: session.policy_revision,
    }));
  };

  const deny = () => {
    submit(() => onRespond({
      session_id: session.session_id,
      decision: "deny",
      approved_scopes: [],
      expected_policy_revision: session.policy_revision,
    }));
  };

  return (
    <AlertDialog open>
      <AlertDialogContent
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          document.getElementById(denyButtonId)?.focus();
        }}
      >
        <AlertDialogHeader>
          <AlertDialogTitle>Incoming remote session</AlertDialogTitle>
          <AlertDialogDescription>
            {canPair
              ? "选择允许的权限。可仅允许本次连接，或配对并记住此控制端。"
              : "选择允许的权限，确认后仅允许本次连接。"}
          </AlertDialogDescription>
        </AlertDialogHeader>

        <dl className="grid gap-2 text-sm">
          <div>
            <dt className="text-muted-foreground">Device ID</dt>
            <dd className="break-all font-mono">{session.peer_device_id}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">Key ID</dt>
            <dd className="break-all font-mono">{session.peer_key_id}</dd>
          </div>
        </dl>

        <div aria-label="Requested permissions">
          {session.requested_scopes.map((scope) => (
            <label className="flex items-center gap-3 py-1" key={scope}>
              <input
                checked={selectedScopes.has(scope)}
                disabled={isSubmitting || isExpired}
                onChange={() => toggleScope(scope)}
                type="checkbox"
              />
              <span className="font-mono text-sm">{scope}</span>
            </label>
          ))}
        </div>

        {canPair && !hasScreenViewing ? (
          <p className="text-muted-foreground text-sm" role="status">
            配对需要允许画面查看（screen.view）。
          </p>
        ) : null}

        {isExpired ? (
          <p className="text-destructive text-sm" role="status">
            This request has expired.
          </p>
        ) : null}

        {submissionError ? (
          <p className="text-destructive text-sm" role="alert">
            {submissionError}
          </p>
        ) : null}

        <AlertDialogFooter>
          {canPair && onPair && <Button type="button" variant="outline"
            disabled={isSubmitting || isExpired || !hasScreenViewing || [...selectedScopes].some(scope => !['screen.view', 'input.pointer', 'input.keyboard'].includes(scope))}
            onClick={() => submit(() => onPair({ session_id: session.session_id,
              approved_scopes: session.requested_scopes.filter(scope => selectedScopes.has(scope)), expected_policy_revision: session.policy_revision }))}>
            配对并记住此控制端
          </Button>}
          <Button
            disabled={isSubmitting}
            id={denyButtonId}
            onClick={deny}
            type="button"
            variant="destructive"
          >
            Deny
          </Button>
          <Button
            disabled={isSubmitting || isExpired || selectedScopes.size === 0}
            onClick={approve}
            type="button"
          >
            在本机确认
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
