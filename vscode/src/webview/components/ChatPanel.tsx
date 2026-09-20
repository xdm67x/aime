import { useEffect, useRef, useState, type FormEvent } from "react";
import type { Beat, TaskEvent } from "../../protocol";
import { postToHost } from "../api";
import type { PulseState } from "../store";

type BeatEntry = {
  role: string;
  content: string;
  model?: string;
  arguments?: string;
  error?: boolean;
  raw_content?: string;
  images?: string[];
  ts?: string;
};

export function ChatPanel({ beat, state }: { beat: Beat | null; state: PulseState }) {
  const [prompt, setPrompt] = useState("");
  const bottomRef = useRef<HTMLDivElement | null>(null);
  const messages = beat ? (state.messages[beat.id] as BeatEntry[] | undefined) : undefined;
  const live = beat ? state.live[beat.id] : undefined;
  const running = beat ? !!state.running[beat.id] : false;
  const error = beat ? (state.error[beat.id] ?? null) : null;

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages?.length, live?.events.length, live?.streaming, error]);

  const send = (e: FormEvent) => {
    e.preventDefault();
    if (!beat || !prompt.trim() || running) return;
    postToHost({ kind: "run-task", beatId: beat.id, prompt, images: [] });
    setPrompt("");
  };

  if (!beat) {
    return (
      <div className="chat-empty">
        <p>Create or select a beat to start a session.</p>
      </div>
    );
  }

  return (
    <div className="chat">
      <div className="chat-header">
        <strong>{beat.name}</strong>
        {beat.description && <span className="chat-desc">{beat.description}</span>}
        {running && (
          <button
            className="cancel-btn"
            onClick={() => postToHost({ kind: "cancel-task", beatId: beat.id })}
          >
            Stop
          </button>
        )}
      </div>
      <div className="transcript">
        {(messages ?? []).map((m, i) => (
          <TranscriptEntry key={i} entry={m} />
        ))}
        {live && <LiveTurn events={live.events} streaming={live.streaming} />}
        {error && <div className="chat-error">{error}</div>}
        <div ref={bottomRef} />
      </div>
      <form className="composer" onSubmit={send}>
        <textarea
          value={prompt}
          placeholder="Ask anything… (/compact to summarize into a new session)"
          onChange={(e) => setPrompt(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              e.currentTarget.form?.requestSubmit();
            }
            if (e.key === "Escape" && running) {
              postToHost({ kind: "cancel-task", beatId: beat.id });
            }
          }}
          rows={3}
        />
        <button type="submit" disabled={running || !prompt.trim()}>
          Send
        </button>
      </form>
    </div>
  );
}

function TranscriptEntry({ entry }: { entry: BeatEntry }) {
  if (entry.role === "user") {
    return (
      <div className="msg user">
        <div className="bubble">{entry.content}</div>
      </div>
    );
  }
  if (entry.role === "tool") {
    return (
      <ToolCard
        name={entry.model ?? "tool"}
        args={entry.arguments ?? ""}
        result={entry.raw_content ?? entry.content}
        error={!!entry.error}
      />
    );
  }
  return (
    <div className="msg assistant">
      <div className="bubble">
        {entry.content}
        {entry.model && <span className="model-tag">{entry.model}</span>}
      </div>
    </div>
  );
}

function ToolCard({
  name,
  args,
  result,
  error,
}: {
  name: string;
  args: string;
  result: string;
  error: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [diff, plain] = splitDiff(result);
  return (
    <div className={`tool-card${error ? " error" : ""}`}>
      <button className="tool-head" onClick={() => setOpen(!open)}>
        <code>
          {name} {truncate(args, 80)}
        </code>
        <span>{error ? "✕ failed" : open ? "▾" : "▸"}</span>
      </button>
      {open && <pre className="tool-body">{diff ? <DiffView diff={diff} /> : plain}</pre>}
    </div>
  );
}

function splitDiff(result: string): [string, string] {
  const i = result.indexOf("\n␛DIFF␛\n");
  if (i === -1) return ["", result];
  return [result.slice(i + 7), result.slice(0, i)];
}

function DiffView({ diff }: { diff: string }) {
  return (
    <code className="diff">
      {diff.split("\n").map((line, i) => {
        const cls = line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "ctx";
        return (
          <span key={i} className={cls}>
            {line}
            {"\n"}
          </span>
        );
      })}
    </code>
  );
}

function LiveTurn({
  events,
  streaming,
}: {
  events: (TaskEvent & { key: number })[];
  streaming: string;
}) {
  return (
    <div className="live">
      {events.map((ev) => {
        if (ev.type === "start") {
          return (
            <div key={ev.key} className="tier-badge">
              {ev.tier} · {ev.model}
            </div>
          );
        }
        if (ev.type === "step") {
          return (
            <div key={ev.key} className="msg assistant">
              <div className="bubble">{ev.text}</div>
            </div>
          );
        }
        if (ev.type === "tool") {
          return (
            <ToolCard
              key={ev.key}
              name={ev.tool}
              args={ev.arguments}
              result={ev.result}
              error={ev.error}
            />
          );
        }
        return null;
      })}
      {streaming && (
        <div className="msg assistant">
          <div className="bubble streaming">{streaming}▍</div>
        </div>
      )}
    </div>
  );
}

function truncate(s: string, max: number): string {
  return s.length > max ? `${s.slice(0, max)}…` : s;
}
