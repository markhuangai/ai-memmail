import type { AppConfig, ProcessedEmail, StatusResponse } from "./types";

export class ApiError extends Error {
  constructor(
    message: string,
    public readonly status: number
  ) {
    super(message);
  }
}

async function requestJson<T>(
  path: string,
  init: RequestInit = {},
  fetchImpl: typeof fetch = fetch
): Promise<T> {
  const response = await fetchImpl(path, {
    credentials: "same-origin",
    headers: {
      "content-type": "application/json",
      ...(init.headers ?? {})
    },
    ...init
  });
  const payload = await response.json().catch(() => ({}));
  if (!response.ok) {
    const message =
      typeof payload.error === "string" ? payload.error : "request failed";
    throw new ApiError(message, response.status);
  }
  return payload as T;
}

export async function loadStatus(fetchImpl?: typeof fetch): Promise<StatusResponse> {
  return requestJson<StatusResponse>("/api/status", {}, fetchImpl);
}

export async function login(
  key: string,
  fetchImpl?: typeof fetch
): Promise<{ authenticated: boolean }> {
  return requestJson(
    "/api/login",
    {
      method: "POST",
      body: JSON.stringify({ key })
    },
    fetchImpl
  );
}

export async function logout(
  fetchImpl?: typeof fetch
): Promise<{ authenticated: boolean }> {
  return requestJson("/api/logout", { method: "POST" }, fetchImpl);
}

export async function loadConfig(
  fetchImpl?: typeof fetch
): Promise<AppConfig> {
  const payload = await requestJson<{ config: AppConfig }>(
    "/api/config",
    {},
    fetchImpl
  );
  return payload.config;
}

export async function loadMessages(
  fetchImpl?: typeof fetch
): Promise<ProcessedEmail[]> {
  const payload = await requestJson<{ messages?: ProcessedEmail[] }>(
    "/api/messages",
    {},
    fetchImpl
  );
  return payload.messages ?? [];
}

export async function saveConfig(
  config: AppConfig,
  fetchImpl?: typeof fetch
): Promise<AppConfig> {
  const payload = await requestJson<{ config: AppConfig }>(
    "/api/config",
    {
      method: "PUT",
      body: JSON.stringify(config)
    },
    fetchImpl
  );
  return payload.config;
}
