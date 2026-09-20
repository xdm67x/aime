import { mkdtempSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { useDbPath } from "../harness/db";
import * as beats from "../harness/beats";
import * as config from "../harness/config";
import { fill, extractTier } from "../harness/harness";
import { unifiedDiff } from "../harness/diff";
import { discover, loadContent } from "../harness/skills";
import { execute } from "../harness/tools";

let tmp: string;

beforeAll(() => {
  tmp = mkdtempSync(path.join(os.tmpdir(), "pulse-vscode-test-"));
  useDbPath(path.join(tmp, "pulse.db"));
});

afterAll(() => {
  rmSync(tmp, { recursive: true, force: true });
});

describe("config", () => {
  it("saves and loads model slots", () => {
    config.saveModelConfig({ classifier: "c", high: "h", base: "b", low: "l" });
    expect(config.getModelConfig()).toEqual({ classifier: "c", high: "h", base: "b", low: "l" });
  });

  it("rejects unknown providers", () => {
    expect(() => config.saveApiKey("nope", "k")).toThrow(/Unknown provider/);
  });

  it("round-trips api keys", () => {
    config.saveApiKey("openrouter", "  sk-123  ");
    expect(config.apiKey("openrouter")).toBe("sk-123");
  });
});

describe("beats", () => {
  it("creates, lists, archives and deletes", () => {
    const b = beats.createBeat("Test beat", "a description", null);
    expect(b.name).toBe("Test beat");
    expect(b.archived).toBe(false);
    expect(beats.listBeats().some((x) => x.id === b.id)).toBe(true);

    expect(() => beats.createBeat("  ", "", null)).toThrow(/cannot be empty/);

    beats.setBeatArchived(b.id, true);
    expect(beats.listBeats().find((x) => x.id === b.id)?.archived).toBe(true);
    expect(beats.deleteBeat(b.id)).toMatch(/Deleted beat/);
    expect(beats.listBeats().some((x) => x.id === b.id)).toBe(false);
  });

  it("appends and reads back messages", () => {
    const b = beats.createBeat("Msgs", "", null);
    expect(beats.getBeatMessages(b.id)).toEqual([]);
    expect(() => beats.getBeatMessages(999999)).toThrow(/Beat not found/);
  });
});

describe("prompt fill", () => {
  it("replaces placeholders without re-expanding values", () => {
    expect(fill("Task:\n{{prompt}}", { prompt: "fix" })).toBe("Task:\nfix");
    expect(fill("{{a}}-{{a}}", { a: "x" })).toBe("x-x");
    expect(fill("{{a}}", { a: "{{b}}" })).toBe("{{b}}");
  });
});

describe("tier extraction", () => {
  it("parses JSON embedded in prose", () => {
    expect(extractTier('Decision:\n{"tier":"low"}\nthanks')).toBe("low");
    expect(extractTier('{"tier":"HIGH"}')).toBe("high");
    expect(extractTier('{"tier":"nope"}')).toBe("base");
    expect(extractTier("no json")).toBe("base");
  });
});

describe("diff", () => {
  it("marks added and removed lines", () => {
    const d = unifiedDiff("a\nb\nc", "a\nB\nc", 100);
    expect(d).toContain("-b");
    expect(d).toContain("+B");
    expect(d).toContain(" a");
  });

  it("returns empty for identical content and caps output", () => {
    expect(unifiedDiff("same\nlines", "same\nlines", 100)).toBe("");
    const before = Array.from({ length: 10 }, (_, i) => String(i + 1)).join("\n");
    const after = before.replace(/10$/, "X");
    const capped = unifiedDiff(before, after, 4);
    expect(capped).toContain("elided");
    expect(capped).toContain("+X");
  });
});

describe("tools", () => {
  it("writes, reads and edits files", () => {
    const file = path.join(tmp, "t.txt");
    execute("write_file", JSON.stringify({ path: file, content: "hello world" }), null);
    expect(execute("read_file", JSON.stringify({ path: file }), null)).toBe("hello world");
    execute(
      "edit_file",
      JSON.stringify({ path: file, old_string: "hello", new_string: "goodbye" }),
      null,
    );
    expect(execute("read_file", JSON.stringify({ path: file }), null)).toBe("goodbye world");
  });

  it("supports line-range reads and edits", () => {
    const file = path.join(tmp, "lines.txt");
    execute("write_file", JSON.stringify({ path: file, content: "a\nb\nc\nd\ne" }), null);
    expect(execute("read_file", JSON.stringify({ path: file, offset: 2, limit: 2 }), null)).toBe(
      "b\nc",
    );
    execute("edit_file", JSON.stringify({ path: file, start_line: 3, new_string: "C!" }), null);
    expect(execute("read_file", JSON.stringify({ path: file }), null)).toBe("a\nb\nC!\nd\ne");
  });

  it("resolves relative paths against the cwd", () => {
    const dir = path.join(tmp, "proj");
    execute("write_file", JSON.stringify({ path: "x.txt", content: "y" }), dir);
    expect(execute("read_file", JSON.stringify({ path: "x.txt" }), dir)).toBe("y");
  });

  it("strips the UI-only diff section", () => {
    const file = path.join(tmp, "diff.txt");
    const out = execute("write_file", JSON.stringify({ path: file, content: "v1" }), null);
    const stripped = execute("read_file", JSON.stringify({ path: file }), null);
    expect(stripped).not.toContain("DIFF");
    expect(out).toContain("Wrote");
  });

  it("reports errors for unknown tools and missing args", () => {
    expect(() => execute("nope", "{}", null)).toThrow(/Unknown tool/);
    expect(() => execute("read_file", "{}", null)).toThrow(/missing/);
  });
});

describe("skills", () => {
  it("discovers nothing outside a home skills dir", () => {
    expect(discover()).toEqual([]);
    expect(() => loadContent("missing")).toThrow(/skill missing/);
  });
});
