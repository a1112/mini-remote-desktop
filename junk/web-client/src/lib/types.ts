export type DeviceInfo = {
  id: string;
  name: string;
  online: boolean;
  kind?: string;
  transports?: string[];
  capabilities?: Record<string, unknown>;
};

export type AppLogLevel = "info" | "warn" | "error";

export type AppLog = {
  ts: number;
  level: AppLogLevel;
  message: string;
};

export type SignalEnvelope = {
  type: string;
  action: string;
  payload?: Record<string, unknown>;
};
