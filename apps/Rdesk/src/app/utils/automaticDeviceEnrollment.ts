/** Only sanitized service codes reach the product; credentials never do. */
const automaticEnrollmentMessages: Record<string, string> = {
  public_auto_enrollment_pending: "正在自动登记并领取设备码，请稍候。",
  public_auto_enrollment_invalid_request: "本机设备登记暂未完成，后台将自动重试。",
  public_auto_enrollment_connection_failed: "无法连接设备登记服务，后台将自动重试。",
  public_auto_enrollment_unavailable: "服务器自动登记暂不可用，后台将自动重试。",
  public_auto_enrollment_rejected: "服务器暂未允许本机自动登记，后台将自动重试。",
  public_auto_enrollment_identity_conflict: "本机已有设备身份需要恢复，请使用设备恢复入口。",
  public_auto_enrollment_rate_limited: "设备登记请求较多，后台将稍后自动重试。",
  public_auto_enrollment_challenge_expired: "本次设备登记已超时，后台将自动重试。",
  public_auto_enrollment_invalid_challenge: "服务器返回的登记信息无效，后台将自动重试。",
  public_auto_enrollment_invalid_response: "服务器返回的设备码无效，后台将自动重试。",
  public_auto_enrollment_signing_failed: "无法验证本机设备身份，请检查本机服务状态。",
};

export function automaticEnrollmentMessage(code: string | null): string | null {
  return code ? automaticEnrollmentMessages[code] ?? null : null;
}

export function automaticEnrollmentLabel(code: string | null): string | null {
  if (!automaticEnrollmentMessage(code)) return null;
  if (code === "public_auto_enrollment_pending") return "正在领取设备码";
  if (code === "public_auto_enrollment_identity_conflict") return "等待恢复已有设备身份";
  return "自动登记暂未完成，正在重试";
}
