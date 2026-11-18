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

const permissionLabels: Record<RemotePermissionScope, string> = {
  "screen.view": "查看本机屏幕",
  "input.pointer": "控制本机鼠标",
  "input.keyboard": "控制本机键盘",
  "clipboard.read": "读取本机剪贴板",
  "clipboard.write": "写入本机剪贴板",
  "file.read": "读取本机文件",
  "file.write": "写入本机文件",
  "audio.listen": "收听本机系统声音",
  "audio.talk": "语音通话",
  "display.switch": "切换显示器",
  "display.multi_view": "同时查看多个显示器",
  "power.restart": "重启本机",
  "power.shutdown": "关闭本机",
  "terminal.open": "打开本机终端",
  "privacy.block_local_input": "屏蔽本机键盘和鼠标操作",
  "privacy.blank_screen": "将本机屏幕置黑",
  "secure_desktop.view": "查看系统安全桌面",
  "secure_desktop.control": "控制系统安全桌面",
};

/**
 * Select the scopes shown in the attended-consent UI before the user edits
 * the request. Development builds intentionally start with every requested
 * scope selected so the browser-control fixture can exercise the complete
 * input path. Release builds keep the conservative screen-only default.
 *
 * `defaultAllRequested` is injectable for tests; the application uses Vite's
 * build-time DEV flag and never derives this policy from request data.
 */
export function defaultSelectedScopes(
  session: RemoteSessionSnapshot | null,
  defaultAllRequested = import.meta.env.DEV,
) {
  const requestedScopes = session?.requested_scopes ?? [];
  if (defaultAllRequested) {
    return new Set<RemotePermissionScope>(requestedScopes);
  }
  return requestedScopes.includes("screen.view")
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
        className="max-h-[calc(100dvh_-_2rem)] overflow-y-auto"
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          document.getElementById(denyButtonId)?.focus();
        }}
      >
        <AlertDialogHeader>
          <AlertDialogTitle>Incoming remote session</AlertDialogTitle>
          <AlertDialogDescription>
            选择允许的权限后，在本机系统窗口中确认请求。
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

        <fieldset>
          <legend className="mb-1 text-sm font-medium">本次请求的权限</legend>
          <div className="max-h-[min(40dvh,16rem)] overflow-y-auto">
            {session.requested_scopes.map((scope) => (
              <label className="flex items-center gap-3 py-1" key={scope}>
                <input
                  checked={selectedScopes.has(scope)}
                  disabled={isSubmitting || isExpired}
                  onChange={() => toggleScope(scope)}
                  type="checkbox"
                />
                <span className="text-sm">{permissionLabels[scope] ?? scope}</span>
              </label>
            ))}
          </div>
        </fieldset>

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
            在本机确认
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
