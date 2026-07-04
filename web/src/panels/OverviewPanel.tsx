import { displaySecret, type ConfigSummary } from "../configModel";
import type { AppConfig } from "../types";

export function Overview({ summary, config }: { summary: ConfigSummary; config: AppConfig }) {
  return (
    <div className="overview-grid">
      <Metric label="Mailboxes" value={`${summary.enabledMailboxes}/${summary.mailboxCount}`} />
      <Metric label="MCP servers" value={String(summary.mcpServerCount)} />
      <Metric label="Avg poll" value={`${summary.averagePollSeconds}s`} />
      <Metric label="Banned senders" value={String(summary.bannedSenderCount)} />
      <section className="panel wide">
        <h2>Runtime</h2>
        <dl className="detail-grid">
          <div>
            <dt>AI model</dt>
            <dd>{config.ai.AI_MODEL}</dd>
          </div>
          <div>
            <dt>AI secret</dt>
            <dd>{displaySecret(config.ai.AI_API_SECRET)}</dd>
          </div>
          <div>
            <dt>Safety prompt</dt>
            <dd>{config.prompts.safety_scan}</dd>
          </div>
          <div>
            <dt>Postgres</dt>
            <dd>{config.database.host}:{config.database.port}</dd>
          </div>
          <div>
            <dt>Log retention</dt>
            <dd>{config.logging.retention_days} days</dd>
          </div>
        </dl>
      </section>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <section className="metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </section>
  );
}
