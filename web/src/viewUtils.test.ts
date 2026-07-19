import { describe, expect, it } from "vitest";
import {
  errorMessage,
  formatTimestamp,
  isSensitiveEnvName,
  messageKey,
  nextEnvKey,
  statusPillClass,
  timestampMs
} from "./viewUtils";
import { sampleMessages } from "./fixtures";

describe("viewUtils", () => {
  it("formats message and environment helpers", () => {
    expect(messageKey(sampleMessages[0])).toBe("support:1:42");
    expect(nextEnvKey({ ENV_VAR_1: "a", ENV_VAR_3: "c" })).toBe("ENV_VAR_4");
    expect(nextEnvKey({ ENV_VAR_2: "b" })).toBe("ENV_VAR_3");
    expect(isSensitiveEnvName("OPENAI_API_KEY")).toBe(true);
    expect(isSensitiveEnvName("DISPLAY_NAME")).toBe(false);
  });

  it("classifies status pill styles", () => {
    expect(statusPillClass("send_failed")).toBe("status-pill danger");
    expect(statusPillClass("processing")).toBe("status-pill pending");
    expect(statusPillClass("replied")).toBe("status-pill success");
    expect(statusPillClass("sent")).toBe("status-pill");
  });

  it("formats dates and fallback errors", () => {
    expect(formatTimestamp("not-a-date")).toBe("not-a-date");
    expect(timestampMs("not-a-date")).toBe(0);
    expect(timestampMs("2026-07-01T00:00:00Z")).toBeGreaterThan(0);
    expect(errorMessage(new Error("bad request"))).toBe("bad request");
    expect(errorMessage("bad request")).toBe("request failed");
  });
});
