import type {
  AppConfig,
  EmailClassificationConfig,
  NewEmailRule,
  PortalConversationDetail,
  PortalConversationSummary,
  PortalSendRequest,
  ProcessedEmail,
  PromptFile,
  StatusResponse,
  ThreadHandoffSummary
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
  limitOrFetch?: number | typeof fetch,
  fetchImpl?: typeof fetch
): Promise<ProcessedEmail[]> {
  const limit = typeof limitOrFetch === "number" ? limitOrFetch : undefined;
  const fetcher = typeof limitOrFetch === "function" ? limitOrFetch : fetchImpl;
  const path =
    limit === undefined
      ? "/api/messages"
      : `/api/messages?limit=${encodeURIComponent(String(limit))}`;
  const payload = await requestJson<{ messages?: ProcessedEmail[] }>(
    path,
    {},
    fetcher
  );
  return payload.messages ?? [];
}

export async function createHandoff(
  runId: string,
  destination: string,
  fetchImpl?: typeof fetch
): Promise<ThreadHandoffSummary | null> {
  const payload = await requestJson<{ handoff?: ThreadHandoffSummary | null }>(
    `/api/messages/${encodeURIComponent(runId)}/handoff`,
    {
      method: "POST",
      body: JSON.stringify({
        request_id: requestId(),
        destination
      })
    },
    fetchImpl
  );
  return payload.handoff ?? null;
}

export async function loadConversations(
  limitOrFetch?: number | typeof fetch,
  fetchImpl?: typeof fetch
): Promise<PortalConversationSummary[]> {
  const limit = typeof limitOrFetch === "number" ? limitOrFetch : undefined;
  const fetcher = typeof limitOrFetch === "function" ? limitOrFetch : fetchImpl;
  const path =
    limit === undefined
      ? "/api/conversations"
      : `/api/conversations?limit=${encodeURIComponent(String(limit))}`;
  const payload = await requestJson<{ conversations?: PortalConversationSummary[] }>(
    path,
    {},
    fetcher
  );
  return payload.conversations ?? [];
}

export async function loadConversation(
  conversationId: string,
  fetchImpl?: typeof fetch
): Promise<PortalConversationDetail> {
  const payload = await requestJson<{ conversation: PortalConversationDetail }>(
    `/api/conversations/${encodeURIComponent(conversationId)}`,
    {},
    fetchImpl
  );
  return payload.conversation;
}

export async function sendPortalMessage(
  conversationId: string,
  request: PortalSendRequest,
  fetchImpl?: typeof fetch
): Promise<PortalConversationDetail> {
  const payload = await requestJson<{ conversation: PortalConversationDetail }>(
    `/api/conversations/${encodeURIComponent(conversationId)}/messages`,
    {
      method: "POST",
      body: JSON.stringify(request)
    },
    fetchImpl
  );
  return payload.conversation;
}

function requestId(): string {
  if (typeof globalThis.crypto?.randomUUID === "function") {
    return globalThis.crypto.randomUUID();
  }

  const bytes = new Uint8Array(16);
  globalThis.crypto.getRandomValues(bytes);
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;

  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0"));
  return `${hex.slice(0, 4).join("")}-${hex.slice(4, 6).join("")}-${hex
    .slice(6, 8)
    .join("")}-${hex.slice(8, 10).join("")}-${hex.slice(10).join("")}`;
}

export function newRequestId(): string {
  return requestId();
}

export async function loadPromptFile(
  path: string,
  fetchImpl?: typeof fetch
): Promise<PromptFile> {
  return requestJson<PromptFile>(promptFilePath(path), {}, fetchImpl);
}

export async function savePromptFile(
  path: string,
  content: string,
  fetchImpl?: typeof fetch
): Promise<PromptFile> {
  return requestJson<PromptFile>(
    promptFilePath(path),
    {
      method: "PUT",
      body: JSON.stringify({ content })
    },
    fetchImpl
  );
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

function promptFilePath(path: string): string {
  return `/api/prompt-file?path=${encodeURIComponent(path)}`;
}
