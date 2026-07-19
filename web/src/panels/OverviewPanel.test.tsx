import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { summarizeConfig } from "../configModel";
import { sampleConfig } from "../fixtures";
import { Overview } from "./OverviewPanel";

describe("Overview", () => {
  it("formats minute and hour uptimes", () => {
    const summary = summarizeConfig(sampleConfig);
    const status = {
      service: "ai-memmail" as const,
      authenticated: true,
      uptime_seconds: 125,
      enabled_mailboxes: 1
    };
    const { rerender } = render(
      <Overview config={sampleConfig} status={status} summary={summary} />
    );

    expect(screen.getByText("2m")).toBeInTheDocument();

    rerender(
      <Overview
        config={sampleConfig}
        status={{ ...status, uptime_seconds: 7200 }}
        summary={summary}
      />
    );
    expect(screen.getByText("2h")).toBeInTheDocument();
  });
});
