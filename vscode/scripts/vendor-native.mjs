import { cpSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const addonDir = join(root, "pulse-node");
const outDir = join(root, "vscode", "native", "pulse-node");

const files = readdirSync(addonDir).filter(
  (f) => f === "index.js" || f === "index.d.ts" || f.endsWith(".node"),
);
if (!files.some((f) => f.endsWith(".node"))) {
  console.error(
    "No .node binding found in pulse-node/ — run `pnpm --dir pulse-node run build` first.",
  );
  process.exit(1);
}

rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });
for (const f of files) cpSync(join(addonDir, f), join(outDir, f));

console.log(`Vendored pulse-node addon into ${outDir}: ${files.join(", ")}`);
