import { useState, type FormEvent } from "react";
import type { Beat } from "../../protocol";
import { postToHost } from "../api";

export function BeatList({
  beats,
  selected,
  running,
  onSelect,
}: {
  beats: Beat[];
  selected: number | null;
  running: Record<number, boolean>;
  onSelect: (beatId: number) => void;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");

  const create = (e: FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    postToHost({ kind: "create-beat", name, description, projectId: null });
    setName("");
    setDescription("");
  };

  const active = beats.filter((b) => !b.archived);
  const archived = beats.filter((b) => b.archived);

  const item = (b: Beat) => (
    <li key={b.id} className={`beat-item${b.id === selected ? " selected" : ""}`}>
      <button className="beat-select" onClick={() => onSelect(b.id)}>
        <span className="beat-name">
          {running[b.id] ? "⏳ " : ""}
          {b.name}
        </span>
        <span className="beat-meta">
          {b.costUsd > 0 && `$${b.costUsd.toFixed(3)}`}{" "}
          {b.promptTokens + b.completionTokens > 0 &&
            `${((b.promptTokens + b.completionTokens) / 1000).toFixed(1)}k tok`}
        </span>
      </button>
      <span className="beat-actions">
        <button
          title={b.archived ? "Unarchive" : "Archive"}
          onClick={() => postToHost({ kind: "archive-beat", beatId: b.id, archived: !b.archived })}
        >
          {b.archived ? "↩" : "📦"}
        </button>
        {b.archived && (
          <button title="Delete" onClick={() => postToHost({ kind: "delete-beat", beatId: b.id })}>
            ✕
          </button>
        )}
      </span>
    </li>
  );

  return (
    <div className="beat-list">
      <form className="new-beat" onSubmit={create}>
        <input
          placeholder="New beat name…"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <input
          placeholder="Description (optional)"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
        />
        <button type="submit">Create</button>
      </form>
      <ul>{active.map(item)}</ul>
      {archived.length > 0 && <h4>Archived</h4>}
      <ul>{archived.map(item)}</ul>
    </div>
  );
}
