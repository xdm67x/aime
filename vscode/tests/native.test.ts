import { mkdtempSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import * as native from "../src/native";

let tmp: string;

beforeAll(() => {
  tmp = mkdtempSync(path.join(os.tmpdir(), "pulse-vscode-test-"));
  process.env.HOME = tmp;
});

afterAll(() => {
  rmSync(tmp, { recursive: true, force: true });
});

describe("native addon loads", () => {
  it("exposes the harness API", () => {
    expect(typeof native.listBeats).toBe("function");
    expect(typeof native.runTask).toBe("function");
    expect(typeof native.cancelCurrent).toBe("function");
  });
});

describe("config", () => {
  it("saves and loads model slots", () => {
    native.saveModelConfig({ classifier: "c", high: "h", base: "b", low: "l" });
    expect(native.getModelConfig()).toEqual({ classifier: "c", high: "h", base: "b", low: "l" });
  });

  it("rejects unknown providers", () => {
    expect(() => native.saveApiKey("nope", "k")).toThrow(/Unknown provider/);
  });

  it("round-trips api keys", () => {
    native.saveApiKey("openrouter", "  sk-123  ");
    expect(native.getApiKey("openrouter")).toBe("sk-123");
  });
});

describe("beats", () => {
  it("creates, lists, archives and deletes", () => {
    const b = native.createBeat("Test beat", "a description", null);
    expect(b.name).toBe("Test beat");
    expect(b.archived).toBe(false);
    expect(native.listBeats().some((x) => x.id === b.id)).toBe(true);
    expect(() => native.createBeat("  ", "", null)).toThrow(/cannot be empty/);
    native.setBeatArchived(b.id, true);
    expect(native.listBeats().find((x) => x.id === b.id)?.archived).toBe(true);
    expect(native.deleteBeat(b.id)).toMatch(/worktree|Beat/i);
    expect(native.listBeats().some((x) => x.id === b.id)).toBe(false);
  });

  it("appends and reads back messages", () => {
    const b = native.createBeat("Msgs", "", null);
    expect(native.getBeatMessages(b.id)).toEqual([]);
    expect(() => native.getBeatMessages(999999)).toThrow(/Beat not found/);
  });
});

describe("usage totals", () => {
  it("returns per-model rows", () => {
    const b = native.createBeat("Usage", "", null);
    expect(native.usageTotals(b.id)).toEqual([]);
  });
});

describe("tools", () => {
  it("writes, reads and edits files", async () => {
    const file = path.join(tmp, "t.txt");
    await native.executeTool(
      "write_file",
      JSON.stringify({ path: file, content: "hello world" }),
      null,
    );
    expect(await native.executeTool("read_file", JSON.stringify({ path: file }), null)).toBe(
      "hello world",
    );
    await native.executeTool(
      "edit_file",
      JSON.stringify({ path: file, old_string: "hello", new_string: "goodbye" }),
      null,
    );
    expect(await native.executeTool("read_file", JSON.stringify({ path: file }), null)).toBe(
      "goodbye world",
    );
  });

  it("supports line-range reads", async () => {
    const file = path.join(tmp, "lines.txt");
    await native.executeTool(
      "write_file",
      JSON.stringify({ path: file, content: "a\nb\nc\nd\ne" }),
      null,
    );
    expect(
      await native.executeTool(
        "read_file",
        JSON.stringify({ path: file, offset: 2, limit: 2 }),
        null,
      ),
    ).toBe("b\nc");
  });

  it("resolves relative paths against the cwd", async () => {
    const dir = path.join(tmp, "proj");
    await native.executeTool("write_file", JSON.stringify({ path: "x.txt", content: "y" }), dir);
    expect(await native.executeTool("read_file", JSON.stringify({ path: "x.txt" }), dir)).toBe("y");
  });

  it("reports errors for unknown tools and missing args", async () => {
    await expect(native.executeTool("nope", "{}", null)).rejects.toThrow(/Unknown tool/);
    await expect(native.executeTool("read_file", "{}", null)).rejects.toThrow(/missing/);
  });
});

describe("diff", () => {
  it("marks added and removed lines", () => {
    const d = native.unifiedDiff("a\nb\nc", "a\nB\nc", 100);
    expect(d).toContain("-b");
    expect(d).toContain("+B");
    expect(d).toContain(" a");
  });

  it("returns empty for identical content and caps output", () => {
    expect(native.unifiedDiff("same\nlines", "same\nlines", 100)).toBe("");
    const before = Array.from({ length: 10 }, (_, i) => String(i + 1)).join("\n");
    const after = before.replace(/10$/, "X");
    const capped = native.unifiedDiff(before, after, 4);
    expect(capped).toContain("elided");
    expect(capped).toContain("+X");
  });

  it("strips the UI-only diff section", () => {
    expect(native.stripDiff("Wrote 2 bytes\n\u001bDIFF\u001b\n-b")).toBe("Wrote 2 bytes");
  });
});

describe("skills", () => {
  it("discovers nothing outside a home skills dir", () => {
    expect(native.discoverSkills()).toEqual([]);
  });
});
