import { appendMessages, open, type BeatEntry } from "./db";

export type Beat = {
  id: number;
  name: string;
  description: string;
  archived: boolean;
  created_at: string;
  cost_usd: number;
  prompt_tokens: number;
  completion_tokens: number;
  project_id: number | null;
  project_name: string | null;
};

const SELECT = `
  SELECT b.id, b.name, b.description, b.archived, b.created_at,
    COALESCE((SELECT SUM(cost_usd) FROM beat_usage WHERE beat_id = b.id), 0.0) AS cost_usd,
    COALESCE((SELECT SUM(prompt_tokens) FROM beat_usage WHERE beat_id = b.id), 0) AS prompt_tokens,
    COALESCE((SELECT SUM(completion_tokens) FROM beat_usage WHERE beat_id = b.id), 0) AS completion_tokens,
    b.project_id, p.name AS project_name
  FROM beats b LEFT JOIN projects p ON p.id = b.project_id`;

interface BeatRow {
  id: number;
  name: string;
  description: string;
  archived: number;
  created_at: string;
  cost_usd: number;
  prompt_tokens: number;
  completion_tokens: number;
  project_id: number | null;
  project_name: string | null;
}

function toBeat(r: BeatRow): Beat {
  return {
    id: r.id,
    name: r.name,
    description: r.description ?? "",
    archived: r.archived !== 0,
    created_at: r.created_at ?? "",
    cost_usd: r.cost_usd ?? 0,
    prompt_tokens: r.prompt_tokens ?? 0,
    completion_tokens: r.completion_tokens ?? 0,
    project_id: r.project_id ?? null,
    project_name: r.project_name ?? null,
  };
}

export function listBeats(): Beat[] {
  return (open().prepare(`${SELECT} ORDER BY b.archived, b.id DESC`).all() as BeatRow[]).map(
    toBeat,
  );
}

function beatById(conn: ReturnType<typeof open>, id: number | bigint): Beat {
  const row = conn.prepare(`${SELECT} WHERE b.id = ?`).get(id) as BeatRow | undefined;
  if (!row) throw new Error("Beat not found");
  return toBeat(row);
}

export function createBeat(name: string, description: string, projectId: number | null): Beat {
  const n = name.trim();
  if (!n) throw new Error("Beat name cannot be empty");
  const conn = open();
  const info = conn
    .prepare("INSERT INTO beats (name, description, project_id) VALUES (?, ?, ?)")
    .run(n, description.trim(), projectId);
  return beatById(conn, info.lastInsertRowid);
}

export function setBeatArchived(id: number, archived: boolean): void {
  open()
    .prepare("UPDATE beats SET archived = ? WHERE id = ?")
    .run(archived ? 1 : 0, id);
}

export function isContextFull(id: number): boolean {
  const row = open().prepare("SELECT context_full FROM beats WHERE id = ?").get(id) as
    | { context_full: number }
    | undefined;
  if (!row) throw new Error("Beat not found");
  return row.context_full !== 0;
}

export function setContextFull(id: number, full: boolean): void {
  open()
    .prepare("UPDATE beats SET context_full = ? WHERE id = ?")
    .run(full ? 1 : 0, id);
}

export function createSummaryBeat(sourceId: number, summary: string): Beat {
  const conn = open();
  const src = conn.prepare("SELECT name, project_id FROM beats WHERE id = ?").get(sourceId) as {
    name: string;
    project_id: number | null;
  };
  const info = conn
    .prepare("INSERT INTO beats (name, description, project_id) VALUES (?, ?, ?)")
    .run(`${src.name} (compacted)`, summary.trim(), src.project_id);
  const newId = Number(info.lastInsertRowid);
  appendMessages(newId, [{ role: "assistant", content: summary }]);
  return beatById(conn, newId);
}

export function getBeatMessages(id: number): BeatEntry[] {
  const row = open().prepare("SELECT messages FROM beats WHERE id = ?").get(id) as
    | { messages: string }
    | undefined;
  if (!row) throw new Error("Beat not found");
  return JSON.parse(row.messages || "[]") as BeatEntry[];
}

export function deleteBeat(id: number): string {
  const conn = open();
  const beat = conn.prepare("SELECT name, archived FROM beats WHERE id = ?").get(id) as
    | { name: string; archived: number }
    | undefined;
  if (!beat) return "Beat not found";
  if (beat.archived === 0) return "Archive the beat first (archived beats can be deleted)";
  conn.prepare("DELETE FROM beat_usage WHERE beat_id = ?").run(id);
  conn.prepare("DELETE FROM beats WHERE id = ?").run(id);
  return `Deleted beat "${beat.name}"`;
}

export function recordUsage(
  beatId: number,
  model: string,
  promptTokens: number,
  completionTokens: number,
  costUsd: number,
): void {
  open()
    .prepare(
      "INSERT INTO beat_usage (beat_id, model, prompt_tokens, completion_tokens, cost_usd) VALUES (?, ?, ?, ?, ?)",
    )
    .run(beatId, model, promptTokens, completionTokens, costUsd);
}

export type UsageTotal = {
  model: string;
  prompt_tokens: number;
  completion_tokens: number;
  cost_usd: number;
};

export function usageTotals(beatId: number): UsageTotal[] {
  return open()
    .prepare(
      "SELECT model, SUM(prompt_tokens) AS prompt_tokens, SUM(completion_tokens) AS completion_tokens, SUM(cost_usd) AS cost_usd FROM beat_usage WHERE beat_id = ? GROUP BY model",
    )
    .all(beatId) as UsageTotal[];
}
