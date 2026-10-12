import { useEffect, useRef, useState } from 'react';
import { ipcListPairedAccessPolicies, ipcRevokePairedAccess } from '../adapters/tauri';
import type { PairedAccessPolicySnapshot } from '../adapters/tauri/types';

export function PairedAccessPolicyCard({ native }: { native: boolean }) {
  const [policies, setPolicies] = useState<PairedAccessPolicySnapshot[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(false);
  const inFlight = useRef(false);
  const generation = useRef(0);
  const run = async (operation: () => Promise<PairedAccessPolicySnapshot[]>) => {
    if (!native || inFlight.current || !alive.current) return;
    inFlight.current = true; setBusy(true); setError(null);
    const request = ++generation.current;
    try { const result = await operation(); if (alive.current && request === generation.current) setPolicies(result); }
    catch (cause) { if (alive.current && request === generation.current) setError(cause instanceof Error ? cause.message : '配对访问请求失败'); }
    finally { if (alive.current && request === generation.current) { inFlight.current = false; setBusy(false); } }
  };
  const refresh = () => run(async () => {
    const result = await ipcListPairedAccessPolicies();
    if (!result.ok) throw new Error(result.error.message);
    return result.value;
  });
  useEffect(() => {
    alive.current = true; inFlight.current = false;
    void refresh();
    return () => { alive.current = false; generation.current += 1; };
  }, [native]);
  const revoke = (policy: PairedAccessPolicySnapshot) => run(async () => {
    const result = await ipcRevokePairedAccess(policy.peer_key_id, policy.revision);
    if (!result.ok) throw new Error(result.error.message);
    return policies.map(current => current.peer_key_id === policy.peer_key_id ? result.value : current);
  });
  return <div className="my-4 rounded-xl border border-gray-500/20 p-3.5">
    <div className="flex items-center justify-between gap-3"><h4 className="text-sm font-medium">已配对控制端</h4>
      <button type="button" onClick={() => void refresh()} disabled={!native || busy} className="rounded border px-3 py-1 text-xs disabled:opacity-50">刷新配对列表</button>
    </div>
    <p className="mt-2 text-xs text-gray-500">配对后可自动确认；临时密码刷新或访问凭据变更后，需重新连接并在本机配对。</p>
    {!native ? <p className="mt-2 text-xs text-gray-500">请在被控设备的客户端中管理和撤销配对访问。</p> : <>
      {busy && <p role="status" className="mt-2 text-xs">正在读取或保存…</p>}
      {error && <p role="alert" className="mt-2 break-words text-xs text-red-500">{error}</p>}
      {!busy && !error && policies.length === 0 && <p className="mt-2 text-xs text-gray-500">暂无配对访问策略</p>}
      <ul className="mt-3 space-y-3">{policies.map(policy => <li key={policy.peer_key_id} className="rounded-lg bg-gray-500/5 p-3 text-xs">
        <p className="break-all font-mono">{policy.peer_key_id}</p>
        <p className="mt-1">{policy.enabled ? '允许免确认访问' : '免确认已失效'}</p>
        <p className="mt-1 break-words text-gray-500">{policy.scopes.join('、')}</p>
        <button type="button" onClick={() => void revoke(policy)} disabled={busy || !policy.enabled}
          className="mt-2 rounded border px-3 py-1.5 text-red-500 disabled:opacity-50">撤销免确认访问</button>
      </li>)}</ul>
    </>}
  </div>;
}
