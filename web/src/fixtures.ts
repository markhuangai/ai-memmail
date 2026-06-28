import type { AppConfig } from "./types";

export const sampleConfig: AppConfig = {
  version: 1,
  database: {
    url: "postgres://ai_memmail:ai_memmail@postgres:5432/ai_memmail"
  },
  logging: {
    level: "info",
    format: "json",
    verbose_actions: true,
    retention_days: 180
  },
  prompts: {
    root: "./prompts",
    safety_scan: "safety-scan.md"
  },
  ai: {
    protocol: "openai",
    AI_API_URL: "https://api.example/v1",
    AI_API_SECRET: "********",
    AI_MODEL: "gpt-test",
    review: {
      enabled: false,
      prompt_path: "outbound-review.md"
    }
  },
  mcp_servers: {
    dense_mem_primary: {
      transport: "stdio",
      command: "npx",
      args: ["-y", "dense-mem-mcp-proxy"],
      env: {
        DENSE_MEM_MCP_URL: "http://dense-mem:8080/mcp",
        DENSE_MEM_API_KEY: "********"
      },
      url: null
    }
  },
  mailboxes: [
    {
      id: "support",
      address: "support@example.com",
      enabled: true,
      poll_interval_seconds: 60,
      safety_forward_to: ["human@example.com"],
      mcp_servers: ["dense_mem_primary"],
      agent: {
        system_prompt_path: "support-agent.md",
        default_forward_to: ["human@example.com"]
      },
      imap: {
        host: "imap.example.com",
        port: 993,
        tls: true,
        username: "support@example.com",
        password: "********",
        folder: "INBOX"
      },
      smtp: {
        host: "smtp.example.com",
        port: 587,
        starttls: true,
        username: "support@example.com",
        password: "********",
        from: "support@example.com"
      }
    }
  ],
  banned_senders: [
    {
      kind: "domain",
      value: "blocked.example",
      reason: "Known prompt-injection campaign"
    }
  ]
};
