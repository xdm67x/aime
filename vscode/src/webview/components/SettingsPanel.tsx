import { useEffect, useState, type FormEvent } from "react";
import type { Model, ModelConfig, Project, Workflow, WorkflowStep } from "../../protocol";
import { postToHost } from "../api";

const PROVIDERS = ["openrouter", "opencode", "litellm"] as const;
const SLOTS: { key: keyof ModelConfig; label: string }[] = [
  { key: "classifier", label: "Classifier (routes prompts)" },
  { key: "high", label: "High tier (agentic + reflexion)" },
  { key: "base", label: "Base tier (agentic)" },
  { key: "low", label: "Low tier (single completion)" },
];
const STEP_KINDS = ["agent", "ask", "reflexion"] as const;

const KIND_HINTS: Record<(typeof STEP_KINDS)[number], string> = {
  agent: "agentic loop with tools",
  ask: "single completion, no tools",
  reflexion: "critique the previous step",
};

const setDefaultWorkflow = (id: string) => postToHost({ kind: "set-default-workflow", id });

export function SettingsPanel({
  models,
  projects,
  config,
  workflows,
  defaultWorkflow,
}: {
  models: Model[];
  projects: Project[];
  config: ModelConfig | null;
  workflows: Workflow[];
  defaultWorkflow: string;
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

  useEffect(() => {
    postToHost({ kind: "list-workflows" });
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

      <WorkflowEditor models={models} workflows={workflows} defaultWorkflow={defaultWorkflow} />

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

/* ---- workflow editor: compose ordered bricks of models + prompts ---- */

function WorkflowEditor({
  models,
  workflows,
  defaultWorkflow,
}: {
  models: Model[];
  workflows: Workflow[];
  defaultWorkflow: string;
}) {
  const [drafts, setDrafts] = useState<Record<string, Workflow[]>>({});
  const list = (id: string) => drafts[id] ?? workflows;
  const setList = (id: string, next: Workflow[]) => setDrafts((d) => ({ ...d, [id]: next }));
  const current = drafts[defaultWorkflow] ?? workflows.find((w) => w.id === defaultWorkflow);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selected = selectedId ? (list(selectedId).find((w) => w.id === selectedId) ?? null) : null;

  const addWorkflow = () => {
    const id = `wf-${Date.now().toString(36)}`;
    const w: Workflow = {
      id,
      name: "New workflow",
      description: "",
      steps: [{ kind: "agent", model: "", prompt: "", label: "Plan" }],
    };
    setList(id, [...list(id), w]);
    setSelectedId(id);
  };

  const patchWorkflow = (id: string, patch: Partial<Workflow>) => {
    setList(
      id,
      list(id).map((w) => {
        if (w.id !== id) return w;
        const next = Object.assign(Object.create(Object.getPrototypeOf(w)), w);
        return Object.assign(next, patch);
      }),
    );
  };

  const removeWorkflow = (id: string) => {
    setList(
      id,
      list(id).filter((w) => w.id !== id),
    );
    if (selectedId === id) setSelectedId(null);
  };

  const moveStep = (id: string, from: number, to: number) => {
    const w = list(id);
    const wf = w.find((x) => x.id === id);
    if (!wf || to < 0 || to >= wf.steps.length) return;
    const steps = [...wf.steps];
    const [step] = steps.splice(from, 1);
    steps.splice(to, 0, step);
    patchWorkflow(id, { steps });
  };

  const patchStep = (id: string, index: number, patch: Partial<WorkflowStep>) => {
    const wf = list(id).find((x) => x.id === id);
    if (!wf) return;
    const steps = wf.steps.map((s, i) => {
      if (i !== index) return s;
      const next = Object.assign(Object.create(Object.getPrototypeOf(s)), s);
      return Object.assign(next, patch);
    });
    patchWorkflow(id, { steps });
  };

  const addStep = (id: string) => {
    const wf = list(id).find((x) => x.id === id);
    if (!wf) return;
    patchWorkflow(id, {
      steps: [...wf.steps, { kind: "agent", model: "", prompt: "", label: "" }],
    });
  };

  const removeStep = (id: string, index: number) => {
    const wf = list(id).find((x) => x.id === id);
    if (!wf || wf.steps.length <= 1) return;
    patchWorkflow(id, { steps: wf.steps.filter((_, i) => i !== index) });
  };

  const save = () => {
    const all = Object.keys(drafts).flatMap((id) => list(id));
    postToHost({ kind: "save-workflows", workflows: all });
  };

  return (
    <section>
      <h3>Workflows</h3>
      <p className="workflow-hint">
        Chain models in order: every brick runs its own model and prompt, the output of one feeding
        the next. The default workflow replaces the classifier routing for new messages.
      </p>
      <div className="workflow-list">
        {Object.keys(drafts).length === 0 && workflows.length === 0 && (
          <p className="workflow-empty">No workflows yet — create one.</p>
        )}
        {(Object.keys(drafts).length > 0 ? Object.keys(drafts) : workflows.map((w) => w.id)).map(
          (id) => {
            const w = list(id).find((x) => x.id === id);
            if (!w) return null;
            return (
              <div key={id} className="workflow-row">
                <input
                  className="workflow-name"
                  value={w.name}
                  onChange={(e) => patchWorkflow(id, { name: e.target.value })}
                />
                <span className="workflow-steps-count">{w.steps.length} steps</span>
                <label className="workflow-default">
                  <input
                    type="radio"
                    name="default-workflow"
                    checked={defaultWorkflow === id}
                    onChange={() => setDefaultWorkflow(id)}
                  />
                  default
                </label>
                <button onClick={() => setSelectedId(selectedId === id ? null : id)}>
                  {selectedId === id ? "Close" : "Edit"}
                </button>
                <button className="danger" onClick={() => removeWorkflow(id)}>
                  ✕
                </button>
              </div>
            );
          },
        )}
      </div>
      <div className="workflow-actions">
        <button onClick={addWorkflow}>+ New workflow</button>
        <button onClick={save}>Save workflows</button>
        {current && <span className="workflow-current">default: {current.name || current.id}</span>}
      </div>
      {selected && (
        <div className="workflow-editor">
          <div className="field">
            <label>
              Description
              <input
                value={selected.description}
                onChange={(e) => patchWorkflow(selected.id, { description: e.target.value })}
              />
            </label>
          </div>
          <p className="field-label">Steps (drag or use ↑/↓ to reorder)</p>
          <StepList
            workflow={selected}
            models={models}
            onMove={(from, to) => moveStep(selected.id, from, to)}
            onPatch={(i, patch) => patchStep(selected.id, i, patch)}
            onAdd={() => addStep(selected.id)}
            onRemove={(i) => removeStep(selected.id, i)}
          />
        </div>
      )}
    </section>
  );
}

function StepList({
  workflow,
  models,
  onMove,
  onPatch,
  onAdd,
  onRemove,
}: {
  workflow: Workflow;
  models: Model[];
  onMove: (from: number, to: number) => void;
  onPatch: (index: number, patch: Partial<WorkflowStep>) => void;
  onAdd: () => void;
  onRemove: (index: number) => void;
}) {
  const [dragging, setDragging] = useState<number | null>(null);

  return (
    <div className="step-list">
      {workflow.steps.map((step, i) => (
        <div
          key={`step-${i}`}
          className="step-brick"
          draggable
          onDragStart={() => setDragging(i)}
          onDragOver={(e) => e.preventDefault()}
          onDrop={() => {
            if (dragging !== null && dragging !== i) onMove(dragging, i);
            setDragging(null);
          }}
        >
          <span className="step-index" title="Drag to reorder">
            {i + 1}
          </span>
          <input
            className="step-label"
            placeholder="Label"
            value={step.label}
            onChange={(e) => onPatch(i, { label: e.target.value })}
          />
          <select
            className="step-kind"
            value={step.kind}
            onChange={(e) => onPatch(i, { kind: e.target.value as WorkflowStep["kind"] })}
          >
            {STEP_KINDS.map((k) => (
              <option key={k} value={k}>
                {k}
              </option>
            ))}
          </select>
          <select
            className="step-model"
            value={step.model}
            onChange={(e) => onPatch(i, { model: e.target.value })}
          >
            <option value="">— model —</option>
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.id}
              </option>
            ))}
          </select>
          <span className="step-kind-hint">{KIND_HINTS[step.kind]}</span>
          <button onClick={() => onMove(i, i - 1)} disabled={i === 0}>
            ↑
          </button>
          <button onClick={() => onMove(i, i + 1)} disabled={i === workflow.steps.length - 1}>
            ↓
          </button>
          <button className="danger" onClick={() => onRemove(i)}>
            ✕
          </button>
          <textarea
            className="step-prompt"
            placeholder="Custom prompt (optional) — {{prompt}} expands to the user message"
            value={step.prompt}
            onChange={(e) => onPatch(i, { prompt: e.target.value })}
          />
        </div>
      ))}
      <button onClick={onAdd}>+ Add step</button>
    </div>
  );
}
