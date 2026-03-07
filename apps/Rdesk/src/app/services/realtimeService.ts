import { invoke } from "@tauri-apps/api/tauri";

export type RealtimeStatus = {
  running: boolean;
  reachable: boolean;
  status: string;
  pid: number | null;
};

export const getRealtimeStatus = async (): Promise<RealtimeStatus> =>
  invoke<RealtimeStatus>("realtime_status");

export const startRealtime = async (): Promise<RealtimeStatus> =>
  invoke<RealtimeStatus>("realtime_start");

export const stopRealtime = async (): Promise<RealtimeStatus> =>
  invoke<RealtimeStatus>("realtime_stop");

export const restartRealtime = async (): Promise<RealtimeStatus> =>
  invoke<RealtimeStatus>("realtime_restart");
