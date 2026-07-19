import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { sampleConfig } from "./fixtures";
import { classificationResponse, jsonResponse } from "./testHelpers";
import type { AppConfig } from "./types";

describe("App interaction guards", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("saves dirty config before refreshing and closes the mobile drawer", async () => {
    let serverConfig: AppConfig = sampleConfig;
    const savedBodies: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (path === "/api/config" && init?.method === "PUT") {
        savedBodies.push(String(init.body));
        serverConfig = JSON.parse(String(init.body)) as AppConfig;
        return jsonResponse({ config: serverConfig });
      }
      if (String(path).startsWith("/api/messages")) {
        return jsonResponse({ messages: [] });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: serverConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^mailboxes$/i }));
    fireEvent.change(screen.getByLabelText(/poll seconds/i), {
      target: { value: "77" }
    });
    fireEvent.click(screen.getByRole("button", { name: /open navigation/i }));
    expect(document.querySelector(".sidebar.open")).not.toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /close navigation overlay/i }));
    expect(document.querySelector(".sidebar.open")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /open navigation/i }));
    fireEvent.click(screen.getByRole("button", { name: /^close navigation$/i }));
    expect(document.querySelector(".sidebar.open")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /open navigation/i }));
    fireEvent.click(screen.getByRole("button", { name: /^history$/i }));
    expect(document.querySelector(".sidebar.open")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /refresh data/i }));
    const dialog = screen.getByRole("dialog", { name: /unsaved config changes/i });
    fireEvent.click(within(dialog).getByRole("button", { name: /keep editing/i }));
    expect(screen.queryByRole("dialog", { name: /unsaved config changes/i })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /refresh data/i }));
    const saveDialog = screen.getByRole("dialog", { name: /unsaved config changes/i });
    fireEvent.click(within(saveDialog).getByRole("button", { name: /save and continue/i }));

    await waitFor(() => expect(savedBodies).toHaveLength(1));
    expect(JSON.parse(savedBodies[0]).mailboxes[0].poll_interval_seconds).toBe(77);
  });

  it("discards dirty config when refresh is confirmed", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation((path) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (String(path).startsWith("/api/messages")) {
        return jsonResponse({ messages: [] });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^settings$/i }));
    fireEvent.change(screen.getByLabelText(/ai model/i), {
      target: { value: "draft-model" }
    });
    fireEvent.click(screen.getByRole("button", { name: /refresh data/i }));
    fireEvent.click(screen.getByRole("button", { name: /discard and continue/i }));

    await waitFor(() => expect(screen.getByLabelText(/ai model/i)).toHaveValue("gpt-test"));
  });

  it("guards prompt edits before opening another prompt or closing the editor", async () => {
    const savedPromptBodies: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (String(path).startsWith("/api/prompt-file") && init?.method === "PUT") {
        savedPromptBodies.push(String(init.body));
        return jsonResponse({
          path: "prompt.md",
          content: JSON.parse(String(init.body)).content as string
        });
      }
      if (String(path).startsWith("/api/prompt-file")) {
        const url = String(path);
        return jsonResponse({
          path: "prompt.md",
          content: url.includes("email-classifier") ? "Classifier prompt content" : "Safety prompt content"
        });
      }
      if (String(path).startsWith("/api/messages")) {
        return jsonResponse({ messages: [] });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^settings$/i }));
    fireEvent.click(screen.getByRole("button", { name: /open safety prompt/i }));
    const safetyEditor = await screen.findByLabelText(/safety prompt content/i);
    fireEvent.change(safetyEditor, { target: { value: "Updated safety prompt" } });

    fireEvent.click(screen.getByRole("button", { name: /open classifier prompt/i }));
    const saveDialog = screen.getByRole("dialog", { name: /unsaved prompt changes/i });
    fireEvent.click(within(saveDialog).getByRole("button", { name: /keep editing/i }));
    expect(screen.queryByRole("dialog", { name: /unsaved prompt changes/i })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /open classifier prompt/i }));
    const secondSaveDialog = screen.getByRole("dialog", { name: /unsaved prompt changes/i });
    fireEvent.click(within(secondSaveDialog).getByRole("button", { name: /save prompt/i }));
    await waitFor(() => expect(savedPromptBodies).toHaveLength(1));
    expect(JSON.parse(savedPromptBodies[0])).toEqual({ content: "Updated safety prompt" });
    expect(await screen.findByLabelText(/classifier prompt content/i)).toHaveValue("Classifier prompt content");

    fireEvent.change(screen.getByLabelText(/classifier prompt content/i), {
      target: { value: "Discard this prompt" }
    });
    fireEvent.click(screen.getByRole("button", { name: /close prompt/i }));
    fireEvent.click(screen.getByRole("button", { name: /discard prompt/i }));
    await waitFor(() => expect(screen.queryByLabelText(/classifier prompt content/i)).not.toBeInTheDocument());

    fireEvent.change(screen.getByLabelText(/outbound review prompt/i, { selector: "input" }), {
      target: { value: "outbound-review-changed.md" }
    });
    fireEvent.click(screen.getByRole("button", { name: /open outbound review prompt/i }));
    expect(await screen.findByLabelText(/outbound review prompt content/i)).toHaveValue("Safety prompt content");
  });

  it("removes a mailbox only after confirmation", async () => {
    const savedBodies: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
      if (path === "/api/status") {
        return jsonResponse({
          service: "ai-memmail",
          authenticated: true,
          uptime_seconds: 3,
          enabled_mailboxes: 1
        });
      }
      if (path === "/api/config" && init?.method === "PUT") {
        savedBodies.push(String(init.body));
        return jsonResponse({ config: JSON.parse(String(init.body)) as AppConfig });
      }
      if (String(path).startsWith("/api/messages")) {
        return jsonResponse({ messages: [] });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^mailboxes$/i }));
    fireEvent.click(screen.getByRole("button", { name: /^remove$/i }));
    fireEvent.click(screen.getByRole("button", { name: /remove mailbox/i }));
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));

    await waitFor(() => expect(savedBodies).toHaveLength(1));
    expect(JSON.parse(savedBodies[0]).mailboxes).toEqual([]);
  });

  it("signs out immediately when there is no dirty config", async () => {
    let authenticated = true;
    vi.spyOn(globalThis, "fetch").mockImplementation((path) => {
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
      if (String(path).startsWith("/api/messages")) {
        return jsonResponse({ messages: [] });
      }
      if (path === "/api/email-classification") {
        return classificationResponse();
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    await screen.findByRole("heading", { name: "Overview" });
    fireEvent.click(screen.getByRole("button", { name: /sign out/i }));
    expect(await screen.findByLabelText(/control panel key/i)).toBeInTheDocument();
  });
});
