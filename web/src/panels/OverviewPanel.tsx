import { displaySecret, type ConfigSummary } from "../configModel";
import type { AppConfig, StatusResponse } from "../types";

export function Overview({
  summary,
  config,
  status
}: {
  summary: ConfigSummary;
  config: AppConfig;
  status: StatusResponse;
}) {
  return (
    <div className="overview-layout">
      <section className="overview-command">
        <div>
          <span className="eyebrow">Runtime console</span>
          <h2>{status.service}</h2>
          <p>
            {summary.enabledMailboxes} enabled mailbox{summary.enabledMailboxes === 1 ? "" : "es"} polling every{" "}
            {summary.averagePollSeconds}s on average.
          </p>
        </div>
        <dl>
          <Metric label="Mailboxes" value={`${summary.enabledMailboxes}/${summary.mailboxCount}`} />
          <Metric label="MCP servers" value={String(summary.mcpServerCount)} />
          <Metric label="Banned senders" value={String(summary.bannedSenderCount)} />
          <Metric label="Uptime" value={formatUptime(status.uptime_seconds)} />
        </dl>
      </section>

      <section className="panel wide runtime-panel">
        <div className="panel-heading">
          <div>
            <h2>Runtime</h2>
            <p>Values shown here come from the loaded config and status endpoint.</p>
          </div>
        </div>
        <dl className="detail-grid">
          <Detail label="AI model" value={config.ai.AI_MODEL} />
          <Detail label="AI secret" value={displaySecret(config.ai.AI_API_SECRET)} />
          <Detail label="Safety prompt" value={config.prompts.safety_scan} />
          <Detail label="Postgres" value={`${config.database.host}:${config.database.port}`} />
          <Detail label="Log retention" value={`${config.logging.retention_days} days`} />
          <Detail label="Log format" value={config.logging.format} />
        </dl>
      </section>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="metric">
      <dt>{label}</dt>
      <strong>{value}</strong>
    </div>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

function formatUptime(seconds: number): string {
  if (seconds < 60) {
    return `${seconds}s`;
  }
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    return `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  return `${hours}h`;
}
