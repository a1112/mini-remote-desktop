import { useCallback, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  FolderOpen,
  RefreshCw,
  RotateCcw,
  Square,
  WifiOff,
  XCircle,
} from "lucide-react";
import { useTheme } from "./ThemeContext";
import * as commands from "../adapters/tauri/commands";
import type {
  ClientDiagnostics,
  RuntimeSnapshot,
  ShellStatusSnapshot,
} from "../adapters/tauri/types";
import {
  failSession,
  recoverSession,
  stopSession,
} from "../services/ipcSessionService";

type PanelState = "checking" | "online" | "offline" | "error";

export function ServiceStatusPanel() {
  const { isDark } = useTheme();
  const [panelState, setPanelState] = useState<PanelState>("checking");
  const [shellStatus, setShellStatus] = useState<ShellStatusSnapshot | null>(null);
  const [runtimeSnapshot, setRuntimeSnapshot] = useState<RuntimeSnapshot | null>(null);
  const [diagnostics, setDiagnostics] = useState<ClientDiagnostics | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busySessionId, setBusySessionId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setMessage(null);

    const [shellResult, runtimeResult, diagnosticsResult] = await Promise.all([
      commands.shellGetStatus(),
      commands.ipcRuntimeSnapshot(),
      commands.getClientDiagnostics(),
    ]);

    if (diagnosticsResult.ok) {
      setDiagnostics(diagnosticsResult.value);
    }

    if (shellResult.ok) {
      setShellStatus(shellResult.value);
      setPanelState(shellResult.value.last_error ? "error" : "online");
    } else {
      setShellStatus(null);
      setPanelState("offline");
      setMessage(shellResult.error.message);
    }

    if (runtimeResult.ok) {
      setRuntimeSnapshot(runtimeResult.value);
    } else {
      setRuntimeSnapshot(null);
      if (shellResult.ok) {
        setPanelState("error");
        setMessage(runtimeResult.error.message);
      }
    }
  }, []);

  useEffect(() => {
    void refresh();
    const interval = window.setInterval(() => void refresh(), 3000);
    return () => window.clearInterval(interval);
  }, [refresh]);

  const sessions = runtimeSnapshot?.sessions ?? [];
  const activeSessions = useMemo(
    () => sessions.filter((session) => session.state !== "closed"),
    [sessions]
  );

  const handleBootstrap = async () => {
    setPanelState("checking");
    const result = await commands.serviceBootstrapIfNeeded();
    if (!result.ok) {
      setPanelState("offline");
      setMessage(result.error.message);
      return;
    }
    await refresh();
  };

  const handleOpenLogs = async () => {
    const result = await commands.openDiagnosticsFolder();
    if (!result.ok) {
      setMessage(result.error.message);
    }
  };

  const handleStopSession = async (sessionId: string) => {
    setBusySessionId(sessionId);
    try {
      await stopSession(sessionId);
      await refresh();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "Stop session failed");
    } finally {
      setBusySessionId(null);
    }
  };

  const handleFailSession = async (sessionId: string) => {
    setBusySessionId(sessionId);
    try {
      await failSession(sessionId, "manual failure from Rdesk diagnostics");
      await refresh();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "Fail session failed");
    } finally {
      setBusySessionId(null);
    }
  };

  const handleRecoverSession = async (sessionId: string) => {
    setBusySessionId(sessionId);
    try {
      await recoverSession(sessionId);
      await refresh();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "Recover session failed");
    } finally {
      setBusySessionId(null);
    }
  };

  const statusTone = getStatusTone(panelState, isDark);
  const StatusIcon = getStatusIcon(panelState);

  return (
    <section
      className={`shrink-0 border-b px-3 py-2 ${
        isDark ? "bg-[#1f1f1f] border-gray-700" : "bg-white border-gray-200"
      }`}
    >
      <div className="flex items-center gap-3">
        <div className={`flex items-center gap-2 min-w-[210px] ${statusTone.text}`}>
          <StatusIcon className="h-4 w-4" />
          <span className="text-sm font-medium">
            {panelState === "online"
              ? "mrd-service online"
              : panelState === "checking"
                ? "checking service"
                : panelState === "error"
                  ? "service needs attention"
                  : "mrd-service offline"}
          </span>
        </div>

        <div className="flex items-center gap-3 text-xs min-w-0 flex-1">
          <span className={isDark ? "text-gray-400" : "text-gray-500"}>
            pid {shellStatus?.service_pid ?? "-"}
          </span>
          <span className={isDark ? "text-gray-400" : "text-gray-500"}>
            active {shellStatus?.active_session_count ?? activeSessions.length}
          </span>
          {diagnostics && (
            <span
              className={`truncate ${isDark ? "text-gray-500" : "text-gray-400"}`}
              title={diagnostics.service_stdout_log}
            >
              {diagnostics.service_stdout_log}
            </span>
          )}
          {message && (
            <span className={`truncate ${statusTone.text}`} title={message}>
              {message}
            </span>
          )}
        </div>

        <IconButton
          title="Refresh service status"
          onClick={() => void refresh()}
          disabled={panelState === "checking"}
          isDark={isDark}
        >
          <RefreshCw className="h-4 w-4" />
        </IconButton>
        <IconButton title="Open logs" onClick={handleOpenLogs} isDark={isDark}>
          <FolderOpen className="h-4 w-4" />
        </IconButton>
        {panelState === "offline" && (
          <IconButton title="Bootstrap service" onClick={handleBootstrap} isDark={isDark}>
            <RotateCcw className="h-4 w-4" />
          </IconButton>
        )}
      </div>

      {sessions.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-2">
          {sessions.slice(0, 4).map((session) => (
            <div
              key={session.session_id}
              className={`flex items-center gap-2 rounded-md border px-2 py-1 text-xs ${
                isDark
                  ? "border-gray-700 bg-[#252525] text-gray-300"
                  : "border-gray-200 bg-gray-50 text-gray-700"
              }`}
            >
              <span className="font-mono">{session.session_id}</span>
              <span className={stateClass(session.state)}>{session.state}</span>
              {session.last_error && (
                <span className="max-w-[180px] truncate text-red-500" title={session.last_error}>
                  {session.last_error}
                </span>
              )}
              <MiniButton
                title="Fail session"
                disabled={busySessionId === session.session_id || session.state === "failed"}
                onClick={() => void handleFailSession(session.session_id)}
              >
                <XCircle className="h-3.5 w-3.5" />
              </MiniButton>
              <MiniButton
                title="Recover session"
                disabled={
                  busySessionId === session.session_id ||
                  !["failed", "closed"].includes(session.state)
                }
                onClick={() => void handleRecoverSession(session.session_id)}
              >
                <RotateCcw className="h-3.5 w-3.5" />
              </MiniButton>
              <MiniButton
                title="Stop session"
                disabled={busySessionId === session.session_id || session.state === "closed"}
                onClick={() => void handleStopSession(session.session_id)}
              >
                <Square className="h-3.5 w-3.5" />
              </MiniButton>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

function getStatusIcon(state: PanelState) {
  if (state === "online") return CheckCircle2;
  if (state === "offline") return WifiOff;
  if (state === "error") return AlertTriangle;
  return RefreshCw;
}

function getStatusTone(state: PanelState, isDark: boolean) {
  if (state === "online") {
    return { text: isDark ? "text-emerald-400" : "text-emerald-600" };
  }
  if (state === "offline") {
    return { text: isDark ? "text-red-400" : "text-red-600" };
  }
  if (state === "error") {
    return { text: isDark ? "text-amber-400" : "text-amber-600" };
  }
  return { text: isDark ? "text-blue-400" : "text-blue-600" };
}

function stateClass(state: string): string {
  if (state === "streaming" || state === "connected") return "text-emerald-500";
  if (state === "failed") return "text-red-500";
  if (state === "closed") return "text-gray-500";
  return "text-amber-500";
}

function IconButton({
  children,
  disabled,
  isDark,
  onClick,
  title,
}: {
  children: ReactNode;
  disabled?: boolean;
  isDark: boolean;
  onClick: () => void;
  title: string;
}) {
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onClick={onClick}
      className={`flex h-8 w-8 items-center justify-center rounded-md transition-colors disabled:opacity-40 ${
        isDark
          ? "text-gray-300 hover:bg-gray-700"
          : "text-gray-600 hover:bg-gray-100"
      }`}
    >
      {children}
    </button>
  );
}

function MiniButton({
  children,
  disabled,
  onClick,
  title,
}: {
  children: ReactNode;
  disabled?: boolean;
  onClick: () => void;
  title: string;
}) {
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onClick={onClick}
      className="flex h-6 w-6 items-center justify-center rounded text-gray-500 transition-colors hover:bg-black/10 hover:text-gray-900 disabled:opacity-30 dark:hover:bg-white/10 dark:hover:text-gray-100"
    >
      {children}
    </button>
  );
}
