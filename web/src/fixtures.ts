import type { AppConfig, EmailClassificationConfig, ProcessedEmail } from "./types";

export const sampleConfig: AppConfig = {
  version: 1,
  database: {
    host: "postgres",
    port: 5432,
    username: "ai_memmail",
    password: "********",
    database: "ai_memmail"
  },
  logging: {
    level: "info",
    format: "json",
    verbose_actions: true,
    retention_days: 180
  },
  prompts: {
    root: "./prompts",
    safety_scan: "safety-scan.md",
    email_classifier: "email-classifier.md",
    rule_action: "rule-action.md"
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
      signature: null,
      accepted_conditions: [],
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
        folder: "INBOX",
        sent_folder: null,
        sent_backfill_days: 30
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

export const sampleMessages: ProcessedEmail[] = [
  {
    run_id: "2e7bcb41-5034-45a4-8135-3c33e6275d67",
    mailbox_id: "support",
    uid_validity: 1,
    uid: 42,
    thread_id: "<42@example.com>",
    message_id: "<42@example.com>",
    in_reply_to: null,
    references: [],
    from_addr: "person@example.com",
    subject: "Pricing question",
    inbound_body: "Can you send the current pricing plan?",
    inbound_body_truncated: false,
    status: "replied",
    safety_category: "safe",
    safety_reason: "routine support request",
    agent_action: "reply",
    agent_safety_notes: "message is safe to answer",
    outbound_action: "reply",
    outbound_recipients: ["person@example.com"],
    outbound_subject: "Re: Pricing question",
    outbound_body: "Thanks for reaching out. The current plan is available.\n\n--\nThis is an automated email reply from ai-memmail. If this needs to be escalated to a human, reply with: escalation to human",
    outbound_body_html: null,
    outbound_body_redacted: false,
    outbound_message_id: "<auto-42@example.com>",
    outbound_reason: "memory supported answer",
    outbound_review_status: "approved",
    outbound_review_reason: "reply matches policy",
    classification_category: "question",
    classification_topics: ["general"],
    classification_reason: "sender asks a concrete question",
    classification_confidence: 91,
    decision_source: "agent",
    matched_rule_id: null,
    matched_rule_name: null,
    matched_rule_goal: null,
    created_at: "2026-07-01 00:00:00+00",
    updated_at: "2026-07-01 00:01:00+00",
    logs: [
      {
        level: "info",
        run_id: "2e7bcb41-5034-45a4-8135-3c33e6275d67",
        action: "processing_claim",
        status: "claimed",
        duration_ms: 1,
        detail: null,
        created_at: "2026-07-01 00:00:01+00"
      },
      {
        level: "info",
        run_id: "2e7bcb41-5034-45a4-8135-3c33e6275d67",
        action: "smtp_send",
        status: "replied",
        duration_ms: 122,
        detail: "memory supported answer",
        created_at: "2026-07-01 00:00:20+00"
      }
    ]
  },
  {
    run_id: "b65f05ba-688c-49be-81af-920141f8a35c",
    mailbox_id: "support",
    uid_validity: 1,
    uid: 43,
    thread_id: "<43@example.com>",
    message_id: "<43@example.com>",
    in_reply_to: null,
    references: [],
    from_addr: "blocked@example.com",
    subject: "Suspicious prompt injection sample",
    inbound_body: "Redacted prompt-injection sample requesting instruction override and secret disclosure.",
    inbound_body_truncated: false,
    status: "quarantined",
    safety_category: "prompt_injection",
    safety_reason: "message contains prompt-injection language",
    agent_action: null,
    agent_safety_notes: null,
    outbound_action: "forward",
    outbound_recipients: ["human@example.com"],
    outbound_subject: "[Potential jailbreak] Suspicious prompt injection sample",
    outbound_body: null,
    outbound_body_html: null,
    outbound_body_redacted: true,
    outbound_message_id: null,
    outbound_reason: "message contains prompt-injection language",
    outbound_review_status: null,
    outbound_review_reason: null,
    classification_category: null,
    classification_topics: [],
    classification_reason: null,
    classification_confidence: null,
    decision_source: null,
    matched_rule_id: null,
    matched_rule_name: null,
    matched_rule_goal: null,
    created_at: "2026-07-01 00:02:00+00",
    updated_at: "2026-07-01 00:03:00+00",
    logs: [
      {
        level: "info",
        run_id: "b65f05ba-688c-49be-81af-920141f8a35c",
        action: "safety_scan",
        status: "prompt_injection",
        duration_ms: 11,
        detail: "message contains prompt-injection language",
        created_at: "2026-07-01 00:02:10+00"
      }
    ]
  }
];

export const sampleClassification: EmailClassificationConfig = {
  categories: [
    {
      id: 1,
      name: "marketing_vendor",
      description: "Paid marketing, growth, SEO, lead-generation, advertising, PR, agency, tool, or vendor service outreach.",
      status: "active",
      source: "seed",
      created_at: "2026-07-01 00:00:00+00",
      updated_at: "2026-07-01 00:00:00+00"
    },
    {
      id: 2,
      name: "question",
      description: "A concrete question about Mark, a project, article, setup, usage, or technical direction.",
      status: "active",
      source: "seed",
      created_at: "2026-07-01 00:00:00+00",
      updated_at: "2026-07-01 00:00:00+00"
    },
    {
      id: 3,
      name: "project_opportunity",
      description: "A collaboration, contribution, integration, partnership, investment, job, speaking, or project opportunity.",
      status: "active",
      source: "seed",
      created_at: "2026-07-01 00:00:00+00",
      updated_at: "2026-07-01 00:00:00+00"
    }
  ],
  topics: [
    {
      id: 1,
      name: "dense_mem",
      description: "Dense-Mem project and memory infrastructure.",
      status: "active",
      source: "seed",
      created_at: "2026-07-01 00:00:00+00",
      updated_at: "2026-07-01 00:00:00+00"
    },
    {
      id: 2,
      name: "ai_memmail",
      description: "ai-memmail workflow and control panel.",
      status: "active",
      source: "seed",
      created_at: "2026-07-01 00:00:00+00",
      updated_at: "2026-07-01 00:00:00+00"
    },
    {
      id: 3,
      name: "general",
      description: "General or unclear topic.",
      status: "active",
      source: "seed",
      created_at: "2026-07-01 00:00:00+00",
      updated_at: "2026-07-01 00:00:00+00"
    }
  ],
  rules: [
    {
      id: 1,
      mailbox_id: "support",
      name: "Auto-decline marketing/vendor outreach",
      category_id: 1,
      category: "marketing_vendor",
      topic_ids: [],
      topics: [],
      action: "reply",
      reply_goal: "Politely thank the sender and decline paid marketing, growth, SEO, lead-generation, advertising, PR, or vendor service offers.",
      enabled: true,
      priority: 100,
      created_at: "2026-07-01 00:00:00+00",
      updated_at: "2026-07-01 00:00:00+00"
    }
  ]
};
