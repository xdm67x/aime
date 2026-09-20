export type DiffLine = "context" | "added" | "removed";

export function unifiedDiff(before: string, after: string, maxLines: number): string {
  const a = before.split("\n");
  const b = after.split("\n");
  const n = a.length;
  const m = b.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () =>
    Array.from<number>({ length: m + 1 }).fill(0),
  );
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const out: { kind: DiffLine; line: string }[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: "context", line: a[i] });
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      out.push({ kind: "removed", line: a[i] });
      i++;
    } else {
      out.push({ kind: "added", line: b[j] });
      j++;
    }
  }
  while (i < n) {
    out.push({ kind: "removed", line: a[i] });
    i++;
  }
  while (j < m) {
    out.push({ kind: "added", line: b[j] });
    j++;
  }
  if (out.every((o) => o.kind === "context")) return "";
  const elided = Math.max(0, out.length - maxLines);
  const kept = out.slice(elided);
  let s = "";
  if (elided > 0) s += `… (${elided} unchanged/earlier lines elided)\n`;
  for (const { kind, line } of kept) {
    s += (kind === "context" ? " " : kind === "added" ? "+" : "-") + line + "\n";
  }
  return s;
}
