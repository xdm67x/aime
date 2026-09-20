import Database from "better-sqlite3";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export type BeatEntry = {
  role: string;
  content: string;
  model?: string;
  arguments?: string;
  error?: boolean;
  raw_content?: string;
  images?: string[];
  ts?: string;
};

let dbPath: string | null = null;

export function dbFile(): string {
  if (!dbPath) {
    dbPath = process.env.PULSE_DB ?? path.join(os.homedir(), ".pulse", "pulse.db");
  }
  return dbPath;
}

export function useDbPath(p: string): void {
  dbPath = p;
}

export function open(): Database.Database {
  const dir = path.dirname(dbFile());
  fs.mkdirSync(dir, { recursive: true });
  const conn = new Database(dbFile());
  conn.pragma("journal_mode = WAL");
  conn.pragma("foreign_keys = ON");
  conn.exec(`
    CREATE TABLE IF NOT EXISTS config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS projects (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      name TEXT NOT NULL UNIQUE,
      path TEXT NOT NULL UNIQUE,
      source TEXT NOT NULL DEFAULT 'local',
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE TABLE IF NOT EXISTS beats (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      name TEXT NOT NULL,
      description TEXT NOT NULL DEFAULT '',
      archived INTEGER NOT NULL DEFAULT 0,
      messages TEXT NOT NULL DEFAULT '[]',
      project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
      context_full INTEGER NOT NULL DEFAULT 0,
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE TABLE IF NOT EXISTS beat_usage (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      beat_id INTEGER NOT NULL REFERENCES beats(id),
      model TEXT NOT NULL,
      prompt_tokens INTEGER NOT NULL DEFAULT 0,
      completion_tokens INTEGER NOT NULL DEFAULT 0,
      cost_usd REAL NOT NULL DEFAULT 0,
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
  `);
  return conn;
}

export function getSetting(key: string): string | null {
  const row = open().prepare("SELECT value FROM config WHERE key = ?").get(key) as
    | { value: string }
    | undefined;
  return row?.value ?? null;
}

export function setSetting(key: string, value: string): void {
  open()
    .prepare(
      "INSERT INTO config (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .run(key, value);
}

export function appendMessages(beatId: number, entries: BeatEntry[]): void {
  const conn = open();
  const row = conn.prepare("SELECT messages FROM beats WHERE id = ?").get(beatId) as
    | { messages: string }
    | undefined;
  if (!row) throw new Error("Beat not found");
  const ts = (conn.prepare(`SELECT datetime('now')`).get() as { "datetime('now')": string })[
    "datetime('now')"
  ];
  const arr: BeatEntry[] = row.messages ? (JSON.parse(row.messages) as BeatEntry[]) : [];
  for (const e of entries) arr.push({ ...e, ts });
  conn.prepare("UPDATE beats SET messages = ? WHERE id = ?").run(JSON.stringify(arr), beatId);
}
