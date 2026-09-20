import fs from "node:fs";
import path from "node:path";
import { unifiedDiff } from "./diff";
import { discover, loadContent, type SkillInfo } from "./skills";

export type ToolStep = {
  tool: string;
  arguments: string;
  result: string;
  error: boolean;
};

const DIFF_MAX_LINES = 200;

export function toolDefinitions(skills: SkillInfo[]): Record<string, unknown>[] {
  const tools: Record<string, unknown>[] = [
    {
      type: "function",
      function: {
        name: "task_complete",
        description:
          "Signal that the task is fully finished and no further work is needed. Call this — and only this — as your final action, once every part of the task is verified done. Include the complete final answer for the user. Do NOT call it while work remains (unverified changes, failing tests, unanswered parts of the request).",
        parameters: {
          type: "object",
          properties: {
            summary: {
              type: "string",
              description:
                "The complete final answer / summary of the work for the user. Markdown allowed.",
            },
          },
          required: ["summary"],
        },
      },
    },
    {
      type: "function",
      function: {
        name: "read_file",
        description:
          "Read the contents of a file at the given path. Optionally read a line range (1-indexed, inclusive).",
        parameters: {
          type: "object",
          properties: {
            path: { type: "string", description: "Path to the file." },
            offset: { type: "integer", description: "First line to read (1-indexed). Optional." },
            limit: {
              type: "integer",
              description: "Number of lines to read from offset. Optional.",
            },
          },
          required: ["path"],
        },
      },
    },
    {
      type: "function",
      function: {
        name: "write_file",
        description: "Write content to a file, creating or overwriting it.",
        parameters: {
          type: "object",
          properties: {
            path: { type: "string", description: "Path to the file." },
            content: { type: "string", description: "Content to write." },
          },
          required: ["path", "content"],
        },
      },
    },
    {
      type: "function",
      function: {
        name: "edit_file",
        description:
          "Edit a file in one of two ways: (a) replace the first occurrence of old_string with new_string, or (b) replace the line range start_line..end_line (1-indexed, inclusive) with new_string.",
        parameters: {
          type: "object",
          properties: {
            path: { type: "string", description: "Path to the file." },
            old_string: { type: "string", description: "Exact text to find (string mode)." },
            new_string: { type: "string", description: "Replacement text." },
            start_line: {
              type: "integer",
              description:
                "First line to replace (1-indexed). Line mode when set; requires new_string, old_string not needed.",
            },
            end_line: {
              type: "integer",
              description:
                "Last line to replace (1-indexed, inclusive). Optional; defaults to start_line.",
            },
          },
          required: ["path", "new_string"],
        },
      },
    },
    {
      type: "function",
      function: {
        name: "grep",
        description:
          "Search files for a regex pattern. Returns matching lines with file paths and line numbers.",
        parameters: {
          type: "object",
          properties: {
            pattern: { type: "string", description: "Regex pattern to search for." },
            path: {
              type: "string",
              description: "Directory or file to search in. Defaults to current directory.",
            },
            glob: { type: "string", description: "Optional file glob filter, e.g. *.rs" },
          },
          required: ["pattern"],
        },
      },
    },
    {
      type: "function",
      function: {
        name: "bash",
        description:
          "Execute a shell command and return its output. Use for search, running scripts, or any command-line task.",
        parameters: {
          type: "object",
          properties: {
            command: { type: "string", description: "Shell command to execute." },
          },
          required: ["command"],
        },
      },
    },
  ];
  for (const skill of skills) {
    tools.push({
      type: "function",
      function: {
        name: `skill_${skill.name}`,
        description: `Load the full instructions for the '${skill.name}' skill. Use when the task matches this skill's description.\n\n${skill.description}`,
        parameters: { type: "object", properties: {} },
      },
    });
  }
  return tools;
}

export function resolve(p: string, cwd: string | null): string {
  if (path.isAbsolute(p) || !cwd) return p;
  return path.join(cwd, p);
}

export function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return `${s.slice(0, max)}\n… (truncated, ${s.length} total chars)`;
}

function diffSuffix(diff: string): string {
  return diff ? `\n␛DIFF␛\n${diff}` : "";
}

export function stripDiff(result: string): string {
  const i = result.indexOf("\n␛DIFF␛\n");
  return i === -1 ? result : result.slice(0, i);
}

export function execute(name: string, argsJson: string, cwd: string | null): string {
  let args: Record<string, any> = {};
  try {
    args = JSON.parse(argsJson);
  } catch {
    /* empty arguments default */
  }
  switch (true) {
    case name === "read_file":
      return readFileTool(args, cwd);
    case name === "write_file":
      return writeFileTool(args, cwd);
    case name === "edit_file":
      return editFileTool(args, cwd);
    case name === "grep":
      return grepTool(args, cwd);
    case name === "bash":
      return bashTool(args, cwd);
    case name.startsWith("skill_"):
      return loadContent(name.slice("skill_".length));
    default:
      throw new Error(`Unknown tool: ${name}`);
  }
}

function readFileTool(args: Record<string, any>, cwd: string | null): string {
  const p = resolve(String(args.path ?? missing("path")), cwd);
  const content = fs.readFileSync(p, "utf8");
  const offset = Math.max(1, Number(args.offset ?? 1));
  const limit = args.limit === undefined ? null : Number(args.limit);
  if (offset === 1 && limit === null) return truncate(content, 50_000);
  const lines = content.split("\n");
  const start = Math.min(offset - 1, lines.length);
  const end = limit === null ? lines.length : Math.min(start + limit, lines.length);
  if (start >= lines.length) {
    throw new Error(`offset ${offset} is past end of file (${lines.length} lines)`);
  }
  return lines.slice(start, end).join("\n");
}

function missing(k: string): never {
  throw new Error(`missing '${k}'`);
}

function writeFileTool(args: Record<string, any>, cwd: string | null): string {
  const p = resolve(String(args.path ?? missing("path")), cwd);
  const content = String(args.content ?? missing("content"));
  fs.mkdirSync(path.dirname(p), { recursive: true });
  const before = fs.existsSync(p) ? fs.readFileSync(p, "utf8") : "";
  fs.writeFileSync(p, content);
  const diff = unifiedDiff(before, content, DIFF_MAX_LINES);
  return `Wrote ${content.length} bytes to ${p}${diffSuffix(diff)}`;
}

function editFileTool(args: Record<string, any>, cwd: string | null): string {
  const p = resolve(String(args.path ?? missing("path")), cwd);
  const next = String(args.new_string ?? missing("new_string"));
  const content = fs.readFileSync(p, "utf8");
  if (args.start_line !== undefined) {
    const start = Math.max(1, Number(args.start_line));
    const end = Math.max(1, Number(args.end_line ?? start));
    if (end < start) throw new Error(`end_line ${end} is before start_line ${start}`);
    const lines = content.split("\n");
    if (start > lines.length) {
      throw new Error(`start_line ${start} is past end of file (${lines.length} lines)`);
    }
    const e = Math.min(end, lines.length);
    const out = [...lines.slice(0, start - 1), ...next.split("\n"), ...lines.slice(e)].join("\n");
    fs.writeFileSync(p, out);
    const after = fs.readFileSync(p, "utf8");
    const diff = unifiedDiff(content, after, DIFF_MAX_LINES);
    return `Replaced lines ${start}-${e} in ${p}${diffSuffix(diff)}`;
  }
  const old = String(args.old_string ?? missing("old_string (or set start_line)"));
  const count = content.split(old).length - 1;
  if (count === 0) throw new Error("old_string not found in file");
  const newContent = content.replace(old, next);
  fs.writeFileSync(p, newContent);
  const diff = unifiedDiff(content, newContent, DIFF_MAX_LINES);
  return `Replaced 1 of ${count} occurrence(s) in ${p}${diffSuffix(diff)}`;
}

function grepTool(args: Record<string, any>, cwd: string | null): string {
  const pattern = String(args.pattern ?? missing("pattern"));
  const target = resolve(String(args.path ?? "."), cwd);
  const glob = args.glob ? String(args.glob) : null;
  const rgArgs = ["--line-number", "--no-heading", "--color", "never"];
  if (glob) rgArgs.push("--glob", glob);
  rgArgs.push(pattern, target);
  try {
    const out = spawnSyncOrNull("rg", rgArgs);
    if (out && out.stdout) return truncate(out.stdout, 10_000);
  } catch {
    /* fall through to grep */
  }
  const grepArgs = ["-rn", "--color=never"];
  if (glob) grepArgs.push("--include", glob);
  grepArgs.push(pattern, target);
  const o = spawnSyncOrNull("grep", grepArgs);
  if (!o) throw new Error("neither rg nor grep is available");
  if (o.stdout) return truncate(o.stdout, 10_000);
  if (o.status === 0) return "(no matches)";
  throw new Error(o.stderr || "grep failed");
}

function spawnSyncOrNull(
  bin: string,
  argv: string[],
): { stdout: string; stderr: string; status: number | null } | null {
  try {
    const { status, stdout, stderr } = require("node:child_process").spawnSync(bin, argv, {
      encoding: "utf8",
      maxBuffer: 16 * 1024 * 1024,
    });
    if (status === null && stdout == null) return null;
    return { stdout: stdout ?? "", stderr: stderr ?? "", status };
  } catch {
    return null;
  }
}

function bashTool(args: Record<string, any>, cwd: string | null): string {
  const command = String(args.command ?? missing("command"));
  const { spawnSync } = require("node:child_process") as typeof import("node:child_process");
  const shell = process.env.SHELL || "/bin/sh";
  const res = spawnSync(shell, ["-l", "-c", command], {
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
    timeout: 30_000,
    cwd: cwd ?? undefined,
  });
  if (res.error) throw new Error(String(res.error));
  if (res.status === 0) return truncate(res.stdout ?? "", 10_000);
  throw new Error(`exit ${res.status}: ${truncate(res.stderr ?? "", 5_000)}`);
}

export { discover };
