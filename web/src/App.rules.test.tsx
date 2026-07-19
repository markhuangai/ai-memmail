import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { sampleClassification, sampleConfig } from "./fixtures";
import { classificationResponse, jsonResponse } from "./testHelpers";

describe("App rules", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("creates email processing rules from the rules tab", async () => {
    const savedRules: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
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
      if (path === "/api/email-rules" && init?.method === "POST") {
        savedRules.push(String(init.body));
        return jsonResponse({
          classification: {
            ...sampleClassification,
            rules: [
              ...sampleClassification.rules,
              {
                ...sampleClassification.rules[0],
                id: 2,
                name: "Decline PR agency outreach",
                reply_goal: "Politely decline paid PR agency services."
              }
            ]
          }
        });
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^rules$/i }));
    expect(screen.getByText("Auto-decline marketing/vendor outreach")).toBeInTheDocument();
    expect(screen.getByText("category:marketing_vendor")).toBeInTheDocument();

    fireEvent.change(screen.getAllByLabelText(/rule name/i)[0], {
      target: { value: "Decline PR agency outreach" }
    });
    fireEvent.change(screen.getAllByLabelText(/response goal/i)[0], {
      target: { value: "Politely decline paid PR agency services." }
    });
    fireEvent.click(screen.getByRole("button", { name: /add rule/i }));

    await waitFor(() => expect(savedRules).toHaveLength(1));
    expect(JSON.parse(savedRules[0])).toMatchObject({
      mailbox_id: "support",
      name: "Decline PR agency outreach",
      category_id: 1,
      topic_ids: [],
      action: "reply",
      reply_goal: "Politely decline paid PR agency services.",
      enabled: true,
      priority: 100
    });
    expect(await screen.findByText("Decline PR agency outreach")).toBeInTheDocument();
  });

  it("manages labels and existing email rules from the rules tab", async () => {
    const categoryBodies: string[] = [];
    const topicBodies: string[] = [];
    const updateBodies: string[] = [];
    const deletedRules: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation((path, init) => {
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
      if (path === "/api/email-categories" && init?.method === "POST") {
        categoryBodies.push(String(init.body));
        return jsonResponse({ classification: sampleClassification });
      }
      if (path === "/api/email-topics" && init?.method === "POST") {
        topicBodies.push(String(init.body));
        return jsonResponse({ classification: sampleClassification });
      }
      if (path === "/api/email-rules/1" && init?.method === "PUT") {
        updateBodies.push(String(init.body));
        return jsonResponse({
          classification: {
            ...sampleClassification,
            rules: [
              {
                ...sampleClassification.rules[0],
                name: "Updated marketing decline",
                topic_ids: [1],
                topics: ["dense_mem"]
              }
            ]
          }
        });
      }
      if (path === "/api/email-rules/1" && init?.method === "DELETE") {
        deletedRules.push(path);
        return jsonResponse({ classification: { ...sampleClassification, rules: [] } });
      }
      return jsonResponse({ config: sampleConfig });
    });

    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /^rules$/i }));
    fireEvent.change(screen.getAllByLabelText(/^name$/i)[0], {
      target: { value: "partner" }
    });
    fireEvent.change(screen.getAllByLabelText(/description/i)[0], {
      target: { value: "Partner outreach" }
    });
    fireEvent.click(screen.getByRole("button", { name: /add category/i }));
    await waitFor(() => expect(categoryBodies).toHaveLength(1));

    fireEvent.change(screen.getAllByLabelText(/^name$/i)[1], {
      target: { value: "project updates" }
    });
    fireEvent.change(screen.getAllByLabelText(/description/i)[1], {
      target: { value: "Project update topic" }
    });
    fireEvent.click(screen.getByRole("button", { name: /add topic/i }));
    await waitFor(() => expect(topicBodies).toHaveLength(1));

    fireEvent.change(screen.getAllByLabelText(/rule name/i)[1], {
      target: { value: "Updated marketing decline" }
    });
    fireEvent.click(screen.getAllByLabelText("dense_mem")[1]);
    fireEvent.click(screen.getByRole("button", { name: /save rule/i }));
    await waitFor(() => expect(updateBodies).toHaveLength(1));
    expect(JSON.parse(updateBodies[0])).toMatchObject({
      name: "Updated marketing decline",
      topic_ids: [1]
    });

    fireEvent.click(screen.getByRole("button", { name: /delete/i }));
    fireEvent.click(screen.getByRole("button", { name: /delete rule/i }));
    await waitFor(() => expect(deletedRules).toEqual(["/api/email-rules/1"]));
    expect(await screen.findByText("No rules configured.")).toBeInTheDocument();
  });
});
