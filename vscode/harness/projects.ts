import fs from "node:fs";
import path from "node:path";
import { open } from "./db";

export type Project = {
  id: number;
  name: string;
  path: string;
  source: string;
  created_at: string;
};

const SELECT = "SELECT id, name, path, source, created_at FROM projects";

function conn() {
  return open();
}

function toProject(r: {
  id: number;
  name: string;
  path: string;
  source: string;
  created_at: string;
}): Project {
  return { id: r.id, name: r.name, path: r.path, source: r.source, created_at: r.created_at };
}

export function listProjects(): Project[] {
  return (conn().prepare(`${SELECT} ORDER BY id DESC`).all() as any[]).map(toProject);
}

function projectById(id: number): Project {
  const row = conn().prepare(`${SELECT} WHERE id = ?`).get(id) as any;
  if (!row) throw new Error("Project not found");
  return toProject(row);
}

export function addProject(input: string): Project {
  const p = input.trim().replace(/\/+$/, "");
  if (!fs.existsSync(p) || !fs.statSync(p).isDirectory()) {
    throw new Error(`Not a directory: ${p}`);
  }
  const name = path.basename(p) || "project";
  try {
    const info = conn()
      .prepare("INSERT INTO projects (name, path, source) VALUES (?, ?, 'local')")
      .run(name, p);
    return projectById(Number(info.lastInsertRowid));
  } catch (e: any) {
    if (String(e).includes("UNIQUE")) throw new Error(`Project already added: ${p}`);
    throw e;
  }
}

export function removeProject(id: number): void {
  conn().prepare("DELETE FROM projects WHERE id = ?").run(id);
}

export function workingDir(beatId: number): string | null {
  const row = conn()
    .prepare(
      "SELECT b.worktree, p.path AS project_path FROM beats b LEFT JOIN projects p ON p.id = b.project_id WHERE b.id = ?",
    )
    .get(beatId) as { worktree: string | null; project_path: string | null } | undefined;
  const p = row?.worktree ?? row?.project_path ?? null;
  if (p) {
    if (!fs.existsSync(p) || !fs.statSync(p).isDirectory()) {
      throw new Error(`Project directory no longer exists: ${p}`);
    }
    return p;
  }
  return null;
}

export function agentsNote(dir: string): string | null {
  for (const name of ["AGENTS.md", "CLAUDE.md", "CONTEXT.md"]) {
    const p = path.join(dir, name);
    if (fs.existsSync(p)) {
      return `Notes from ${name} in the working directory:\n\n${fs.readFileSync(p, "utf8")}`;
    }
  }
  return null;
}
