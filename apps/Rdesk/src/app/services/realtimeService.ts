import { invoke } from "@tauri-apps/api/tauri";

export type RealtimeStatus = {
  running: boolean;
  reachable: boolean;
  status: string;
  pid: number | null;
};

export type NvdecCapabilityProbe = {
  codec: string;
  bit_depth_minus8: number;
  chroma_format: number;
  runtime_supported: boolean;
  runtime_reason: string;
  wired_supported: boolean;
  wired_reason: string;
};

export type NvdecRuntimeProbe = {
  backend: string;
  summary: string;
  checked_items: string[];
  capability_probes: NvdecCapabilityProbe[];
};

export type DecoderPolicy = "auto" | "software" | "nvdec";

export type DecodePolicyResponse = {
  decode_policy: DecoderPolicy;
};

export const getRealtimeStatus = async (): Promise<RealtimeStatus> =>
  invoke<RealtimeStatus>("realtime_status");

export const getNvdecRuntimeProbe = async (): Promise<NvdecRuntimeProbe> =>
  invoke<NvdecRuntimeProbe>("nvdec_runtime_probe");

export const getDecodePolicy = async (): Promise<DecodePolicyResponse> =>
  invoke<DecodePolicyResponse>("decode_policy");

export const setDecodePolicy = async (
  decodePolicy: DecoderPolicy
): Promise<DecodePolicyResponse> =>
  invoke<DecodePolicyResponse>("set_decode_policy", {
    decodePolicy,
  });

export const startRealtime = async (): Promise<RealtimeStatus> =>
  invoke<RealtimeStatus>("realtime_start");

export const stopRealtime = async (): Promise<RealtimeStatus> =>
  invoke<RealtimeStatus>("realtime_stop");

export const restartRealtime = async (): Promise<RealtimeStatus> =>
  invoke<RealtimeStatus>("realtime_restart");
