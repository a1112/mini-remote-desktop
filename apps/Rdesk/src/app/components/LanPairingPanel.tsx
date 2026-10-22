import { useCallback, useEffect, useId, useRef, useState } from "react";
import { ipcApproveLanPairing, ipcListLanPairingCandidates, ipcRefreshLanDiscovery } from "../adapters/tauri";
import type { LanPairingCandidate } from "../adapters/tauri/types";
import {
  AlertDialog, AlertDialogContent, AlertDialogDescription, AlertDialogFooter,
  AlertDialogHeader, AlertDialogTitle,
} from "./ui/alert-dialog";
import { Button } from "./ui/button";

export interface LanPairingPanelProps {
  isDark?: boolean;
  onPaired?: () => void | Promise<void>;
}

function candidateError(candidate: LanPairingCandidate, now: number): string | null {
  if (!candidate.candidate_id || !candidate.device_id || !candidate.device_name ||
      !candidate.discovery_endpoint || !/^[a-f0-9]{64}$/.test(candidate.peer_key_id) ||
      !/^(0|[1-9][0-9]{0,19})$/.test(candidate.key_epoch) ||
      BigInt(candidate.key_epoch) > 18446744073709551615n ||
      !Number.isSafeInteger(candidate.expires_at_ms) ||
      candidate.permission_ceiling.length !== 1 || candidate.permission_ceiling[0] !== "screen.view") {
    return "候选信息无效，请刷新后重新核对。";
  }
  return now >= candidate.expires_at_ms ? "候选已过期，请刷新后重新核对。" : null;
}

function sameCandidateBinding(left: LanPairingCandidate, right: LanPairingCandidate): boolean {
  return left.candidate_id === right.candidate_id && left.device_id === right.device_id &&
    left.device_name === right.device_name && left.peer_key_id === right.peer_key_id &&
    left.key_epoch === right.key_epoch && left.discovery_endpoint === right.discovery_endpoint &&
    left.permission_ceiling.length === right.permission_ceiling.length &&
    left.permission_ceiling.every((scope, index) => scope === right.permission_ceiling[index]);
}

function CandidateIdentity({ candidate }: { candidate: LanPairingCandidate }) {
  return (
    <dl className="grid gap-1 text-sm">
      <div><dt className="text-muted-foreground">机器名</dt><dd>{candidate.device_name}</dd></div>
      <div><dt className="text-muted-foreground">设备 ID</dt><dd className="break-all font-mono">{candidate.device_id}</dd></div>
      <div><dt className="text-muted-foreground">局域网地址</dt><dd className="font-mono">{candidate.discovery_endpoint}</dd></div>
      <div><dt className="text-muted-foreground">公钥指纹（请与对端核对）</dt><dd className="break-all font-mono">{candidate.peer_key_id}</dd></div>
      <div><dt className="text-muted-foreground">密钥版本</dt><dd className="font-mono">{candidate.key_epoch}</dd></div>
      <div><dt className="text-muted-foreground">候选有效期至</dt><dd>{Number.isSafeInteger(candidate.expires_at_ms) ? new Date(candidate.expires_at_ms).toLocaleString() : "无效"}</dd></div>
    </dl>
  );
}

export function LanPairingPanel({ isDark = false, onPaired }: LanPairingPanelProps) {
  const [candidates, setCandidates] = useState<LanPairingCandidate[]>([]);
  const [selected, setSelected] = useState<LanPairingCandidate | null>(null);
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [submissionError, setSubmissionError] = useState<string | null>(null);
  const [confirmationWarning, setConfirmationWarning] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const mounted = useRef(false);
  const submittingRef = useRef(false);
  const readingRef = useRef(false);
  const selectedRef = useRef<LanPairingCandidate | null>(null);
  const querySequence = useRef(0);
  const cancelButtonId = useId();

  const load = useCallback(async (refreshDiscovery = false) => {
    if (readingRef.current || submittingRef.current || !mounted.current) return;
    readingRef.current = true;
    const sequence = ++querySequence.current;
    setLoading(true);
    setError(null);
    try {
      if (refreshDiscovery) {
        const discovery = await ipcRefreshLanDiscovery();
        if (!discovery.ok) throw new Error(discovery.error.message);
      }
      const result = await ipcListLanPairingCandidates();
      if (!result.ok) throw new Error(result.error.message);
      if (mounted.current && querySequence.current === sequence) {
        setCandidates(result.value);
        const current = selectedRef.current;
        if (current) {
          const matches = result.value.filter(candidate => candidate.candidate_id === current.candidate_id);
          const fresh = matches.length === 1 ? matches[0] : undefined;
          const observedAt = Date.now();
          if (fresh && sameCandidateBinding(current, fresh) &&
              !candidateError(current, observedAt) && !candidateError(fresh, observedAt)) {
            const renewed = { ...current, expires_at_ms: fresh.expires_at_ms };
            selectedRef.current = renewed;
            setSelected(renewed);
          } else {
            selectedRef.current = null;
            setSelected(null);
            setSubmissionError(null);
            setConfirmationWarning("配对候选已失效，请重新核对机器名、地址和公钥指纹。");
          }
        }
      }
    } catch (failure) {
      if (mounted.current && querySequence.current === sequence) {
        setCandidates([]);
        selectedRef.current = null;
        setSelected(null);
        setConfirmationWarning(null);
        setError(failure instanceof Error ? failure.message : "无法读取配对候选。");
      }
    } finally {
      readingRef.current = false;
      if (mounted.current && querySequence.current === sequence) {
        setLoading(false);
        setNow(Date.now());
      }
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void load();
    const timer = window.setInterval(() => setNow(Date.now()), 500);
    const refreshTimer = window.setInterval(() => void load(), 2_000);
    return () => {
      mounted.current = false;
      ++querySequence.current;
      window.clearInterval(timer);
      window.clearInterval(refreshTimer);
    };
  }, [load]);

  const approve = async () => {
    const current = selectedRef.current;
    if (!current || submittingRef.current || readingRef.current) return;
    const invalid = candidateError(current, Date.now());
    if (invalid) {
      setNow(Date.now());
      setSubmissionError(invalid);
      return;
    }
    submittingRef.current = true;
    setSubmitting(true);
    setSubmissionError(null);
    try {
      const result = await ipcApproveLanPairing({
        candidate_id: current.candidate_id,
        device_id: current.device_id,
        peer_key_id: current.peer_key_id,
        key_epoch: current.key_epoch,
        discovery_endpoint: current.discovery_endpoint,
        permission_ceiling: ["screen.view"],
      });
      if (!result.ok) throw new Error(result.error.message);
      if (!mounted.current) return;
      selectedRef.current = null;
      setSelected(null);
      setCandidates(candidates => candidates.filter(candidate => candidate.candidate_id !== current.candidate_id));
      setStatus(`${current.device_name} 已配对，仅屏幕查看。每次远程会话仍需在被控设备上确认。`);
      const discovery = await ipcRefreshLanDiscovery();
      if (!mounted.current) return;
      if (!discovery.ok) setError(`配对已成功，局域网状态刷新失败：${discovery.error.message}`);
      try { await onPaired?.(); }
      catch { if (mounted.current) setError("配对已成功，设备列表刷新失败，请手动刷新。"); }
    } catch (failure) {
      if (mounted.current) setSubmissionError(failure instanceof Error ? failure.message : "配对失败，请重新核对候选。");
    } finally {
      submittingRef.current = false;
      if (mounted.current) setSubmitting(false);
    }
  };

  const selectedError = selected ? candidateError(selected, now) : null;
  return (
    <section aria-label="局域网首次配对" className={`mb-5 rounded-xl border p-4 ${isDark ? "border-gray-700 bg-[#232323] text-gray-100" : "border-gray-200 bg-white text-gray-900"}`}>
      <div className="flex items-center justify-between gap-3 mb-2">
        <h2 className="font-medium">局域网首次配对</h2>
        <Button variant="outline" disabled={loading || submitting} onClick={() => void load(true)}>刷新配对候选</Button>
      </div>
      <p className="text-sm text-muted-foreground mb-3">请与对端核对机器名、地址和完整公钥指纹。配对仅允许屏幕查看；每次远程会话仍需在被控设备上确认。</p>
      {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}
      {confirmationWarning ? <p role="alert" className="text-sm text-destructive">{confirmationWarning}</p> : null}
      {status ? <p role="status" className="text-sm">{status}</p> : null}
      {loading ? <p role="status">正在读取已验签的局域网设备…</p> : null}
      {!loading && !error && candidates.length === 0 ? <p className="text-sm text-muted-foreground">暂无可配对设备。请确保对端客户端运行，并刷新候选。</p> : null}
      <div className="grid gap-3 mt-3">
        {candidates.map(candidate => {
          const invalid = candidateError(candidate, now);
          return (
            <article key={candidate.candidate_id} className="rounded-lg border p-3">
              <CandidateIdentity candidate={candidate} />
              {invalid ? <p className="text-sm text-destructive mt-2">{invalid}</p> : null}
              <Button className="mt-3" disabled={!!invalid || loading || submitting} onClick={() => { selectedRef.current = candidate; setSelected(candidate); setSubmissionError(null); setConfirmationWarning(null); }}>核对并配对 {candidate.device_name}</Button>
            </article>
          );
        })}
      </div>
      {selected ? (
        <AlertDialog open>
          <AlertDialogContent onOpenAutoFocus={event => { event.preventDefault(); document.getElementById(cancelButtonId)?.focus(); }}>
            <AlertDialogHeader>
              <AlertDialogTitle>确认局域网设备配对</AlertDialogTitle>
              <AlertDialogDescription>请核对对端公钥指纹。此次配对仅允许屏幕查看，每次远程会话仍需在被控设备上确认。</AlertDialogDescription>
            </AlertDialogHeader>
            <CandidateIdentity candidate={selected} />
            <p className="text-sm">权限上限：<span className="font-mono">screen.view</span>（屏幕查看）</p>
            {selectedError && !submissionError ? <p role="status" className="text-sm text-destructive">{selectedError}</p> : null}
            {submissionError ? <p role="alert" className="text-sm text-destructive">{submissionError}</p> : null}
            <AlertDialogFooter>
              <Button id={cancelButtonId} variant="outline" disabled={submitting} onClick={() => { selectedRef.current = null; setSelected(null); }}>取消</Button>
              <Button disabled={!!selectedError || submitting || loading} onClick={() => void approve()}>{submitting ? "正在配对…" : "确认配对，仅屏幕查看"}</Button>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </section>
  );
}
