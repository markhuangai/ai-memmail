import type { ProcessedEmail } from "./types";

export function messageKey(message: ProcessedEmail): string {
  return `${message.mailbox_id}:${message.uid_validity}:${message.uid}`;
}

export function nextEnvKey(env: Record<string, string>): string {
  let index = Object.keys(env).length + 1;
  let key = `ENV_VAR_${index}`;
  while (Object.prototype.hasOwnProperty.call(env, key)) {
    index += 1;
    key = `ENV_VAR_${index}`;
  }
  return key;
}

export function isSensitiveEnvName(name: string): boolean {
  return /(?:KEY|SECRET|TOKEN|PASSWORD)$/i.test(name);
}

export function statusPillClass(status: string): string {
  if (status.includes("failed")) {
    return "status-pill danger";
  }
  if (status === "processing") {
    return "status-pill pending";
  }
  if (["replied", "forwarded", "noop", "quarantined"].includes(status)) {
    return "status-pill success";
  }
  return "status-pill";
}

export function formatTimestamp(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }
  return parsed.toLocaleString();
}

export function timestampMs(value: string): number {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? 0 : parsed.getTime();
}

export function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : "request failed";
}
