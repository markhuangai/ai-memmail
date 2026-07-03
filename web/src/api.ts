import type {
  AppConfig,
  EmailClassificationConfig,
  NewEmailRule,
  ProcessedEmail,
  StatusResponse
} from "./types";

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

export async function loadEmailClassification(
  fetchImpl?: typeof fetch
): Promise<EmailClassificationConfig> {
  const payload = await requestJson<{ classification: EmailClassificationConfig }>(
    "/api/email-classification",
    {},
    fetchImpl
  );
  return payload.classification;
}

export async function createEmailCategory(
  name: string,
  description: string,
  fetchImpl?: typeof fetch
): Promise<EmailClassificationConfig> {
  return mutateEmailClassification(
    "/api/email-categories",
    { name, description },
    "POST",
    fetchImpl
  );
}

export async function createEmailTopic(
  name: string,
  description: string,
  fetchImpl?: typeof fetch
): Promise<EmailClassificationConfig> {
  return mutateEmailClassification(
    "/api/email-topics",
    { name, description },
    "POST",
    fetchImpl
  );
}

export async function createEmailRule(
  rule: NewEmailRule,
  fetchImpl?: typeof fetch
): Promise<EmailClassificationConfig> {
  return mutateEmailClassification("/api/email-rules", rule, "POST", fetchImpl);
}

export async function updateEmailRule(
  id: number,
  rule: NewEmailRule,
  fetchImpl?: typeof fetch
): Promise<EmailClassificationConfig> {
  return mutateEmailClassification(
    `/api/email-rules/${id}`,
    rule,
    "PUT",
    fetchImpl
  );
}

export async function deleteEmailRule(
  id: number,
  fetchImpl?: typeof fetch
): Promise<EmailClassificationConfig> {
  const payload = await requestJson<{ classification: EmailClassificationConfig }>(
    `/api/email-rules/${id}`,
    { method: "DELETE" },
    fetchImpl
  );
  return payload.classification;
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

async function mutateEmailClassification(
  path: string,
  body: unknown,
  method: "POST" | "PUT",
  fetchImpl?: typeof fetch
): Promise<EmailClassificationConfig> {
  const payload = await requestJson<{ classification: EmailClassificationConfig }>(
    path,
    {
      method,
      body: JSON.stringify(body)
    },
    fetchImpl
  );
  return payload.classification;
}
