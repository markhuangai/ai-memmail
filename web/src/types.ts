export type AiProtocol = "openai" | "anthropic";
export type McpTransport = "stdio" | "streamable_http";
export type BannedSenderKind = "email" | "domain";
export type EmailSignatureFormat = "plain_text" | "html";

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
  email_classifier: string;
  rule_action: string;
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
  signature?: EmailSignatureConfig | null;
  accepted_conditions: AcceptedCondition[];
  mcp_servers: string[];
  agent: AgentConfig;
  imap: ImapConfig;
  smtp: SmtpConfig;
}

export interface EmailSignatureConfig {
  format: EmailSignatureFormat;
  content: string;
}

export interface AcceptedCondition {
  recipients: string[];
  subject_regex: string[];
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
  sent_folder: string | null;
  sent_backfill_days: number;
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

export interface ProcessedEmail {
  run_id: string;
  mailbox_id: string;
  uid_validity: number;
  uid: number;
  thread_id: string;
  message_id?: string | null;
  in_reply_to?: string | null;
  references: string[];
  from_addr: string;
  subject: string;
  inbound_body?: string | null;
  inbound_body_truncated: boolean;
  status: string;
  safety_category?: string | null;
  safety_reason?: string | null;
  agent_action?: string | null;
  agent_safety_notes?: string | null;
  outbound_action?: string | null;
  outbound_recipients: string[];
  outbound_subject?: string | null;
  outbound_body?: string | null;
  outbound_body_html?: string | null;
  outbound_body_redacted: boolean;
  outbound_message_id?: string | null;
  outbound_reason?: string | null;
  outbound_review_status?: string | null;
  outbound_review_reason?: string | null;
  classification_category?: string | null;
  classification_topics: string[];
  classification_reason?: string | null;
  classification_confidence?: number | null;
  decision_source?: string | null;
  matched_rule_id?: number | null;
  matched_rule_name?: string | null;
  matched_rule_goal?: string | null;
  created_at: string;
  updated_at: string;
  logs: ProcessedEmailLog[];
  handoff?: ThreadHandoffSummary | null;
}

export interface ThreadHandoffSummary {
  state: string;
  destination: string;
  remote_target: string;
  last_error?: string | null;
  updated_at: string;
}

export interface ProcessedEmailLog {
  level: string;
  run_id: string;
  action: string;
  status: string;
  duration_ms: number;
  detail?: string | null;
  created_at: string;
}

export interface PortalConversationSummary {
  conversation_id: string;
  mailbox_id: string;
  thread_id: string;
  subject: string;
  revision: number;
  last_message_at: string;
  latest_sender: string;
  latest_status: string;
  remote_reply_to?: string | null;
  unsafe_reply_requires_confirmation: boolean;
  source_conversation_id?: string | null;
  handoff?: ThreadHandoffSummary | null;
}

export interface PortalConversationDetail {
  conversation: PortalConversationSummary;
  messages: PortalTimelineMessage[];
  quote_text: string;
  quote_html: string;
}

export interface PortalTimelineMessage {
  id: string;
  direction: "inbound" | "outbound";
  kind: "inbound" | "ai_reply" | "portal_reply" | "portal_forward" | string;
  status: string;
  from_addr: string;
  to_recipients: string[];
  cc_recipients: string[];
  bcc_recipients?: string[];
  subject: string;
  text_body?: string | null;
  html_body?: string | null;
  body_truncated: boolean;
  message_id?: string | null;
  in_reply_to?: string | null;
  references: string[];
  safety_category?: string | null;
  created_at: string;
}

export interface PortalSendRequest {
  request_id: string;
  thread_revision: number;
  action: "reply" | "forward";
  authored_text: string;
  authored_html?: string | null;
  to_recipients?: string[];
  cc_recipients?: string[];
  bcc_recipients?: string[];
  unsafe_confirmed?: boolean;
}

export interface PromptFile {
  path: string;
  content: string;
}

export interface EmailCategory {
  id: number;
  name: string;
  description: string;
  status: string;
  source: string;
  created_at: string;
  updated_at: string;
}

export interface EmailTopic {
  id: number;
  name: string;
  description: string;
  status: string;
  source: string;
  created_at: string;
  updated_at: string;
}

export type EmailRuleAction = "reply" | "forward" | "noop";

export interface EmailRule {
  id: number;
  mailbox_id: string;
  name: string;
  category_id: number;
  category: string;
  topic_ids: number[];
  topics: string[];
  action: EmailRuleAction;
  reply_goal: string;
  enabled: boolean;
  priority: number;
  created_at: string;
  updated_at: string;
}

export interface NewEmailRule {
  mailbox_id: string;
  name: string;
  category_id: number;
  topic_ids: number[];
  action: EmailRuleAction;
  reply_goal: string;
  enabled: boolean;
  priority: number;
}

export interface EmailClassificationConfig {
  categories: EmailCategory[];
  topics: EmailTopic[];
  rules: EmailRule[];
}
