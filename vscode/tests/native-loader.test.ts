import { describe, expect, it } from "vitest";
import * as native from "../src/native";

describe("native loader", () => {
  it("resolves the workspace pulse-node package", () => {
    expect(typeof native.requireNative().runTask).toBe("function");
  });
});
