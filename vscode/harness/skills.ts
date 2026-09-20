import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export type SkillInfo = {
  name: string;
  description: string;
  path: string;
};

function skillsDir(): string {
  return path.join(os.homedir(), ".agents", "skills");
}

export function discover(): SkillInfo[] {
  const dir = skillsDir();
  const skills: SkillInfo[] = [];
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return skills;
  }
  for (const entry of entries) {
    const md = path.join(dir, entry.name, "SKILL.md");
    if (entry.isDirectory() && fs.existsSync(md)) {
      const s = parseFrontmatter(md);
      if (s) skills.push(s);
    }
  }
  skills.sort((a, b) => a.name.localeCompare(b.name));
  return skills;
}

export function loadContent(name: string): string {
  const p = path.join(skillsDir(), name, "SKILL.md");
  try {
    return fs.readFileSync(p, "utf8");
  } catch (e) {
    throw new Error(`skill ${name}: ${e}`);
  }
}

function parseFrontmatter(file: string): SkillInfo | null {
  const content = fs.readFileSync(file, "utf8");
  const lines = content.split("\n");
  if ((lines[0] ?? "").trim() !== "---") return null;
  const fm: string[] = [];
  for (let i = 1; i < lines.length; i++) {
    if (lines[i].trim() === "---") break;
    fm.push(lines[i]);
  }
  const name = yamlField(fm, "name");
  if (!name) return null;
  return {
    name,
    description: yamlField(fm, "description") ?? "",
    path: file,
  };
}

function yamlField(lines: string[], field: string): string | null {
  const prefix = `${field}:`;
  for (let i = 0; i < lines.length; i++) {
    const trimmed = lines[i].trimStart();
    if (!trimmed.startsWith(prefix)) continue;
    const value = trimmed.slice(prefix.length).trim();
    if (value && !value.startsWith(">") && !value.startsWith("|")) {
      return unquote(value);
    }
    if (value.startsWith(">") || value.startsWith("|")) {
      const literal = value.startsWith("|");
      const parts: string[] = [];
      for (const next of lines.slice(i + 1)) {
        if (!next.trim()) {
          parts.push("");
          continue;
        }
        if (next.startsWith(" ") || next.startsWith("\t")) {
          parts.push(next.trim());
        } else {
          break;
        }
      }
      while (parts.length && !parts[parts.length - 1]) parts.pop();
      return parts.join(literal ? "\n" : " ");
    }
    return unquote(value);
  }
  return null;
}

function unquote(v: string): string {
  return v.replace(/^['"]|['"]$/g, "");
}
