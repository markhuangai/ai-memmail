import { describe, expect, it } from "vitest";
import { errorMessage, nextEnvKey } from "./viewUtils";

describe("viewUtils", () => {
  it("chooses the next available env placeholder key", () => {
    expect(nextEnvKey({ ENV_VAR_1: "one", ENV_VAR_2: "two" })).toBe("ENV_VAR_3");
  });

  it("formats unknown failures with a default message", () => {
    expect(errorMessage("failed")).toBe("request failed");
  });
});
