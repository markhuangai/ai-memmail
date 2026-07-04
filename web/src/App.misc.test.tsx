import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { sampleConfig } from "./fixtures";
import { classificationResponse, jsonResponse } from "./testHelpers";

describe("App safety and tabs", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("adds and removes banned senders", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation((path) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^safety$/i }));
    fireEvent.click(screen.getByRole("button", { name: /add/i }));
    expect(screen.queryByText("bad@example.com")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText(/ban kind/i), {
      target: { value: "domain" }
    });
    fireEvent.change(screen.getByLabelText(/ban value/i), {
      target: { value: "bad.example" }
    });
    fireEvent.change(screen.getByLabelText(/ban reason/i), {
      target: { value: "jailbreak" }
    });
    fireEvent.click(screen.getByRole("button", { name: /add/i }));

    expect(screen.getByText("bad.example")).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: /remove/i })[0]);
    expect(screen.queryByText("blocked.example")).not.toBeInTheDocument();
  });

  it("renders MCP and settings tabs", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation((path) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /mcp servers/i }));
    expect(screen.getByText("dense_mem_primary")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^settings$/i }));
    expect(screen.getByLabelText(/ai model/i)).toHaveValue("gpt-test");
    expect(screen.getByLabelText(/postgres host/i)).toHaveValue("postgres");
  });
});
