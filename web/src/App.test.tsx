import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { sampleConfig, sampleMessages } from "./fixtures";
import { classificationResponse, jsonResponse } from "./testHelpers";

describe("App", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("logs in and renders overview metrics", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementationOnce(() =>
        jsonResponse({
          service: "ai-memmail",
          authenticated: false,
          uptime_seconds: 0,
          enabled_mailboxes: 0
        })
      )
      .mockImplementationOnce(() => jsonResponse({ authenticated: true }))
      .mockImplementationOnce(() =>
        jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        })
      )
      .mockImplementationOnce(() => jsonResponse({ config: sampleConfig }))
      .mockImplementationOnce(() => jsonResponse({ messages: sampleMessages }))
      .mockImplementationOnce(() => classificationResponse());

    render(<App />);

    fireEvent.change(await screen.findByLabelText(/control panel key/i), {
      target: { value: "panel-key" }
    });
    fireEvent.click(screen.getByRole("button", { name: /login/i }));

    expect(await screen.findByText("MCP servers")).toBeInTheDocument();
    expect(screen.getByText("1/1")).toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledTimes(6);
  });

  it("shows save errors and protects draft config on sign out", async () => {
    let authenticated = true;
    vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (path === "/api/logout") {
        authenticated = false;
        return jsonResponse({ authenticated: false });
      }
      if (path === "/api/config" && init?.method === "PUT") {
        return jsonResponse({ error: "invalid config" }, { status: 400 });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    await screen.findByText("Runtime");
    fireEvent.click(screen.getByRole("button", { name: /^settings$/i }));
    fireEvent.change(await screen.findByLabelText(/ai model/i), {
      target: { value: "broken-model" }
    });
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent("invalid config");

    fireEvent.click(screen.getByRole("button", { name: /sign out/i }));
    expect(screen.getByRole("dialog", { name: /unsaved config changes/i })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /discard and continue/i }));
    expect(await screen.findByLabelText(/control panel key/i)).toBeInTheDocument();
  });
});
