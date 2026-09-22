import { useEffect, useState, type FormEvent } from "react";
import type { Model, ModelConfig, Project } from "../../protocol";
import { postToHost } from "../api";

const PROVIDERS = ["openrouter", "opencode", "litellm"] as const;
const SLOTS: { key: keyof ModelConfig; label: string }[] = [
  { key: "classifier", label: "Classifier (routes prompts)" },
  { key: "high", label: "High tier (agentic + reflexion)" },
  { key: "base", label: "Base tier (agentic)" },
  { key: "low", label: "Low tier (single completion)" },
];

export function SettingsPanel({
  models,
  projects,
  config,
}: {
  models: Model[];
  projects: Project[];
  config: ModelConfig | null;
}) {
  const [edits, setEdits] = useState<Partial<ModelConfig>>({});
  const [keys, setKeys] = useState<Record<string, string>>({});
  const slots: ModelConfig = {
    ...(config ?? { classifier: "", high: "", base: "", low: "" }),
    ...edits,
  };

  useEffect(() => {
    for (const p of PROVIDERS) {
      const handler = (e: MessageEvent) => {
        const msg = e.data;
        if (msg?.kind === "api-key" && msg.provider === p) {
          setKeys((k) => ({ ...k, [p]: msg.key ?? "" }));
        }
      };
      window.addEventListener("message", handler);
      postToHost({ kind: "get-api-key", provider: p });
      return () => window.removeEventListener("message", handler);
    }
  }, []);

  const saveConfig = (e: FormEvent) => {
    e.preventDefault();
    postToHost({ kind: "save-model-config", config: slots });
  };

  const addProject = async () => {
    postToHost({ kind: "pick-folder" });
    const handler = (e: MessageEvent) => {
      if (e.data?.kind === "folder-picked" && e.data.path) {
        postToHost({ kind: "add-project", path: e.data.path });
        window.removeEventListener("message", handler);
      }
    };
    window.addEventListener("message", handler);
  };

  return (
    <div className="settings">
      <section>
        <h3>API keys</h3>
        {PROVIDERS.map((p) => (
          <div key={p} className="field">
            <label>{p}</label>
            <input
              type="password"
              placeholder={`${p} API key`}
              value={keys[p] ?? ""}
              onChange={(e) => setKeys((k) => ({ ...k, [p]: e.target.value }))}
              onBlur={() => postToHost({ kind: "save-api-key", provider: p, key: keys[p] ?? "" })}
            />
          </div>
        ))}
      </section>

      <section>
        <h3>Model slots</h3>
        <form onSubmit={saveConfig}>
          {SLOTS.map(({ key, label }) => (
            <div key={key} className="field">
              <label>{label}</label>
              <select
                value={slots[key]}
                onChange={(e) => setEdits((s) => ({ ...s, [key]: e.target.value }))}
              >
                <option value="">— not set —</option>
                {models.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.id}
                  </option>
                ))}
              </select>
            </div>
          ))}
          <button type="submit">Save model slots</button>
        </form>
      </section>

      <section>
        <h3>Projects</h3>
        <ul className="project-list">
          {projects.map((p) => (
            <li key={p.id}>
              <span title={p.path}>{p.name}</span>
              <button onClick={() => postToHost({ kind: "remove-project", projectId: p.id })}>
                ✕
              </button>
            </li>
          ))}
        </ul>
        <button onClick={addProject}>Add local folder…</button>
      </section>
    </div>
  );
}
