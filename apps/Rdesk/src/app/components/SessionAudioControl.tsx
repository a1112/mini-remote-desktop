import { useEffect, useRef, useState } from "react";
import { Volume2, VolumeX } from "lucide-react";
import { ipcGetSessionAudio, ipcSetSessionAudioMuted, type SessionAudioSnapshot } from "../adapters/tauri";

export function SessionAudioControl({ sessionId, authorized }: { sessionId: string; authorized: boolean }) {
  const [snapshot, setSnapshot] = useState<SessionAudioSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  useEffect(() => {
    generation.current += 1;
    setSnapshot(null);
    setError(null);
    setBusy(false);
    if (!authorized) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      const result = await ipcGetSessionAudio(sessionId);
      if (cancelled) return;
      if (result.ok && result.value.session_id === sessionId) {
        setSnapshot(result.value);
        setError(result.value.last_error);
      } else if (!result.ok) { setError(result.error.message); }
      timer = setTimeout(() => void refresh(), 1000);
    }
    void refresh();
    return () => { cancelled = true; generation.current += 1; if (timer !== undefined) clearTimeout(timer); };
  }, [sessionId, authorized]);

  const ready = authorized && !error && snapshot !== null && ["starting", "listening", "playing"].includes(snapshot.status);
  const label = !authorized ? "声音未授权" : error ? "声音不可用" : snapshot?.muted ? "声音已静音" : snapshot?.status === "playing" ? "远程声音" : "等待远程声音";
  async function toggle() {
    if (!snapshot || busy || !ready) return;
    setBusy(true);
    const currentGeneration = generation.current;
    try {
      const result = await ipcSetSessionAudioMuted(sessionId, !snapshot.muted);
      if (generation.current !== currentGeneration) return;
      if (result.ok && result.value.session_id === sessionId) { setSnapshot(result.value); setError(result.value.last_error); }
      else if (!result.ok) { setError(result.error.message); }
    } finally { if (generation.current === currentGeneration) setBusy(false); }
  }
  return (
    <button type="button" aria-label={label} aria-pressed={snapshot?.muted ?? false}
      title={error ?? (authorized ? "切换远程系统声音静音" : "请在被控端授权会话的 audio.listen 权限")}
      disabled={!ready || busy} onClick={() => void toggle()}
      className="inline-flex h-9 items-center gap-2 rounded-md border border-white/10 px-3 text-[11px] text-slate-200 hover:bg-white/10 disabled:opacity-50">
      {snapshot?.muted || !ready ? <VolumeX className="h-3.5 w-3.5" /> : <Volume2 className="h-3.5 w-3.5" />}
      {label}
    </button>
  );
}
