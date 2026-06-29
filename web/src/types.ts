export type AiProtocol = "openai" | "anthropic";
export type McpTransport = "stdio" | "streamable_http";
export type BannedSenderKind = "email" | "domain";

export interface AppConfig {
  version: number;
  database: DatabaseConfig;
  logging: LoggingConfig;
  prompts: PromptConfig;
  ai: AiConfig;
  mcp_servers: Record<string, McpServerConfig>;
  mailboxes: MailboxConfig[];
  banned_senders: BannedSenderConfig[];
}

export interface DatabaseConfig {
  host: string;
  port: number;
  username: string;
  password: string;
  database: string;
}

export interface LoggingConfig {
  level: "debug" | "info" | "warn" | "error";
  format: "json" | "pretty";
  verbose_actions: boolean;
  retention_days: number;
}

export interface PromptConfig {
  root: string;
  safety_scan: string;
}

export interface AiConfig {
  protocol: AiProtocol;
  AI_API_URL: string;
  AI_API_SECRET: string;
  AI_MODEL: string;
  review: ReviewConfig;
}

export interface ReviewConfig {
  enabled: boolean;
  prompt_path: string;
}

export interface McpServerConfig {
  transport: McpTransport;
  command?: string | null;
  args: string[];
  env: Record<string, string>;
  url?: string | null;
}

export interface MailboxConfig {
  id: string;
  address: string;
  enabled: boolean;
  poll_interval_seconds: number;
  safety_forward_to: string[];
  mcp_servers: string[];
  agent: AgentConfig;
  imap: ImapConfig;
  smtp: SmtpConfig;
}

export interface AgentConfig {
  system_prompt_path: string;
  default_forward_to: string[];
}

export interface ImapConfig {
  host: string;
  port: number;
  tls: boolean;
  username: string;
  password: string;
  folder: string;
}

export interface SmtpConfig {
  host: string;
  port: number;
  starttls: boolean;
  username: string;
  password: string;
  from: string;
}

export interface BannedSenderConfig {
  kind: BannedSenderKind;
  value: string;
  reason: string;
}

export interface StatusResponse {
  service: "ai-memmail";
  authenticated: boolean;
  uptime_seconds: number;
  enabled_mailboxes: number;
}
