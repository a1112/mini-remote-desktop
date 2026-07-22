import { useEffect, useId, useRef, useState } from "react";

import type {
  ConsentResponse,
  RemotePermissionScope,
  RemoteSessionSnapshot,
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

  const submit = (response: ConsentResponse) => {
    if (submittingRef.current) {
      return;
    }

    submittingRef.current = true;
    setIsSubmitting(true);
    setSubmissionError(null);
    let submission: Promise<void>;
    try {
      submission = onRespond(response);
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
    submit({
      session_id: session.session_id,
      decision: "approve",
      approved_scopes: session.requested_scopes.filter((scope) =>
        selectedScopes.has(scope),
      ),
      expected_policy_revision: session.policy_revision,
    });
  };

  const deny = () => {
    submit({
      session_id: session.session_id,
      decision: "deny",
      approved_scopes: [],
      expected_policy_revision: session.policy_revision,
    });
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
            Review the requesting device and choose the permissions to allow.
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
            Allow selected permissions
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
