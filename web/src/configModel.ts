import type {
  AppConfig,
  BannedSenderConfig,
  MailboxConfig,
  McpServerConfig
} from "./types";

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
      folder: "INBOX"
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
