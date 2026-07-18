import type {
  AppConfig,
  BannedSenderConfig,
  EmailSignatureConfig,
  MailboxConfig,
  McpServerConfig
} from "./types";

export const AUTOMATED_REPLY_NOTICE =
  "This automated reply was sent on Mark's behalf. If this needs Mark's attention, reply with: escalation to human";

export const SIGNATURE_SAMPLE_REPLY =
  "Thanks for reaching out. I will follow up with the details.";

export const DEFAULT_PLAIN_SIGNATURE = "--\nMark";
export const DEFAULT_HTML_SIGNATURE = "<p><strong>Mark</strong></p>";

export interface ConfigSummary {
  mailboxCount: number;
  enabledMailboxes: number;
  mcpServerCount: number;
  bannedSenderCount: number;
  averagePollSeconds: number;
}

export function summarizeConfig(config: AppConfig): ConfigSummary {
  const enabled = config.mailboxes.filter((mailbox) => mailbox.enabled);
  const totalPoll = enabled.reduce(
    (sum, mailbox) => sum + mailbox.poll_interval_seconds,
    0
  );
  return {
    mailboxCount: config.mailboxes.length,
    enabledMailboxes: enabled.length,
    mcpServerCount: Object.keys(config.mcp_servers).length,
    bannedSenderCount: config.banned_senders.length,
    averagePollSeconds: enabled.length ? Math.round(totalPoll / enabled.length) : 0
  };
}

export function mailboxRouteLabel(mailbox: MailboxConfig): string {
  const mcpCount = mailbox.mcp_servers.length;
  const reviewerCount = mailbox.safety_forward_to.length;
  return `${mcpCount} MCP / ${reviewerCount} reviewer${reviewerCount === 1 ? "" : "s"}`;
}

export function updateMailbox(
  config: AppConfig,
  mailboxId: string,
  updater: (mailbox: MailboxConfig) => MailboxConfig
): AppConfig {
  return {
    ...config,
    mailboxes: config.mailboxes.map((mailbox) =>
      mailbox.id === mailboxId ? updater(mailbox) : mailbox
    )
  };
}

export function addMailbox(config: AppConfig): AppConfig {
  const id = nextUniqueKey(
    "mailbox",
    new Set(config.mailboxes.map((mailbox) => mailbox.id))
  );
  const address = `${id}@example.com`;
  const mailbox: MailboxConfig = {
    id,
    address,
    enabled: false,
    poll_interval_seconds: 60,
    safety_forward_to: ["review@example.com"],
    signature: null,
    accepted_conditions: [],
    mcp_servers: [],
    agent: {
      system_prompt_path: "support-agent.md",
      default_forward_to: ["review@example.com"]
    },
    imap: {
      host: "imap.example.com",
      port: 993,
      tls: true,
      username: address,
      password: "",
      folder: "INBOX",
      sent_folder: null,
      sent_backfill_days: 30
    },
    smtp: {
      host: "smtp.example.com",
      port: 587,
      starttls: true,
      username: address,
      password: "",
      from: address
    }
  };
  return {
    ...config,
    mailboxes: [...config.mailboxes, mailbox]
  };
}

export function signaturePreviewHtml(signature: EmailSignatureConfig | null | undefined): string {
  const reply = plainTextToHtml(SIGNATURE_SAMPLE_REPLY);
  if (!signature) {
    return `${reply}<br><br>--<br>${plainTextToHtml(AUTOMATED_REPLY_NOTICE)}`;
  }
  if (signature.format === "html") {
    return `${reply}<br><br>${signature.content.trimEnd()}`;
  }
  return `${reply}<br><br>${plainTextToHtml(signature.content.trimEnd())}`;
}

export function plainTextToHtml(value: string): string {
  return escapeHtml(value).replace(/\r\n|\r|\n/g, "<br>");
}

export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function removeMailbox(config: AppConfig, mailboxId: string): AppConfig {
  return {
    ...config,
    mailboxes: config.mailboxes.filter((mailbox) => mailbox.id !== mailboxId)
  };
}

export function setMailboxScalar<K extends keyof MailboxConfig>(
  config: AppConfig,
  mailboxId: string,
  key: K,
  value: MailboxConfig[K]
): AppConfig {
  return updateMailbox(config, mailboxId, (mailbox) => ({
    ...mailbox,
    [key]: value
  }));
}

export function setListFromText(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

export function listToText(values: string[]): string {
  return values.join(", ");
}

export function setLinesFromText(value: string): string[] {
  return value
    .split("\n")
    .map((item) => item.trim())
    .filter(Boolean);
}

export function listToLines(values: string[]): string {
  return values.join("\n");
}

export function textToEnv(value: string): Record<string, string> {
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .reduce<Record<string, string>>((env, line) => {
      const separatorIndex = line.indexOf("=");
      if (separatorIndex <= 0) {
        return env;
      }
      const key = line.slice(0, separatorIndex).trim();
      const nextValue = line.slice(separatorIndex + 1).trim();
      if (key) {
        env[key] = nextValue;
      }
      return env;
    }, {});
}

export function envToText(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([key, value]) => `${key}=${value}`)
    .join("\n");
}

export function addMcpServer(config: AppConfig): AppConfig {
  const name = nextUniqueKey("dense_mem", new Set(Object.keys(config.mcp_servers)));
  return {
    ...config,
    mcp_servers: {
      ...config.mcp_servers,
      [name]: {
        transport: "stdio",
        command: "npx",
        args: ["-y", "dense-mem-mcp-proxy"],
        env: {},
        url: null
      }
    }
  };
}

export function updateMcpServer(
  config: AppConfig,
  name: string,
  updater: (server: McpServerConfig) => McpServerConfig
): AppConfig {
  const current = config.mcp_servers[name];
  if (!current) {
    return config;
  }
  return {
    ...config,
    mcp_servers: {
      ...config.mcp_servers,
      [name]: updater(current)
    }
  };
}

export function renameMcpServer(
  config: AppConfig,
  name: string,
  nextName: string
): AppConfig {
  const trimmedName = nextName.trim();
  if (
    !trimmedName ||
    trimmedName === name ||
    !config.mcp_servers[name] ||
    config.mcp_servers[trimmedName]
  ) {
    return config;
  }
  return {
    ...config,
    mcp_servers: Object.fromEntries(
      Object.entries(config.mcp_servers).map(([serverName, server]) => [
        serverName === name ? trimmedName : serverName,
        server
      ])
    ),
    mailboxes: config.mailboxes.map((mailbox) => ({
      ...mailbox,
      mcp_servers: mailbox.mcp_servers.map((serverName) =>
        serverName === name ? trimmedName : serverName
      )
    }))
  };
}

export function removeMcpServer(config: AppConfig, name: string): AppConfig {
  const { [name]: _removed, ...remainingServers } = config.mcp_servers;
  return {
    ...config,
    mcp_servers: remainingServers,
    mailboxes: config.mailboxes.map((mailbox) => ({
      ...mailbox,
      mcp_servers: mailbox.mcp_servers.filter((server) => server !== name)
    }))
  };
}

export function addBannedSender(
  config: AppConfig,
  sender: BannedSenderConfig
): AppConfig {
  const next = config.banned_senders.filter(
    (entry) => !(entry.kind === sender.kind && entry.value === sender.value)
  );
  return {
    ...config,
    banned_senders: [...next, sender]
  };
}

export function removeBannedSender(
  config: AppConfig,
  sender: BannedSenderConfig
): AppConfig {
  return {
    ...config,
    banned_senders: config.banned_senders.filter(
      (entry) => !(entry.kind === sender.kind && entry.value === sender.value)
    )
  };
}

export function displaySecret(value: string): string {
  return value === "********" ? "configured" : value ? "set" : "missing";
}

function nextUniqueKey(prefix: string, used: Set<string>): string {
  let index = used.size + 1;
  let key = `${prefix}_${index}`;
  while (used.has(key)) {
    index += 1;
    key = `${prefix}_${index}`;
  }
  return key;
}
