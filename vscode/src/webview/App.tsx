import { useCallback, useEffect, useMemo, useState } from "react";
import { postToHost, useHostMessage } from "./api";
import { usePulseStore } from "./store";
import { BeatList } from "./components/BeatList";
import { ChatPanel } from "./components/ChatPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import "./style.css";

type Tab = "chat" | "settings";

export function App() {
  const { state, dispatch } = usePulseStore();
  const [tab, setTab] = useState<Tab>("chat");

  useEffect(() => {
    if (state.toast === null) return;
    const t = setTimeout(() => dispatch({ type: "toast", text: null }), 2500);
    return () => clearTimeout(t);
  }, [state.toast, dispatch]);

  useEffect(() => {
    if (state.selectedBeat === null) return;
    postToHost({ kind: "get-beat-messages", beatId: state.selectedBeat });
    postToHost({ kind: "beat-usage-totals", beatId: state.selectedBeat });
  }, [state.selectedBeat]);

  const onHostMessage = useCallback(
    (msg: import("../protocol").HostToWebview) => {
      switch (msg.kind) {
        case "beats":
          dispatch({ type: "beats", beats: msg.beats });
          break;
        case "beat-messages":
          dispatch({ type: "beat-messages", beatId: msg.beatId, messages: msg.messages });
          break;
        case "projects":
          dispatch({ type: "projects", projects: msg.projects });
          break;
        case "models":
          dispatch({ type: "models", models: msg.models });
          break;
        case "model-config":
          dispatch({ type: "model-config", config: msg.config });
          break;
        case "workflows":
          dispatch({
            type: "workflows",
            workflows: msg.workflows,
            defaultId: msg.defaultId,
          });
          break;
        case "task-event":
          dispatch({ type: "task-event", beatId: msg.beatId, ev: msg.ev });
          break;
        case "task-result":
          dispatch({ type: "task-result", beatId: msg.beatId, result: msg.result });
          break;
        case "task-error":
          dispatch({ type: "task-error", beatId: msg.beatId, error: msg.error });
          break;
        case "usage-totals":
          dispatch({ type: "usage-totals", beatId: msg.beatId, totals: msg.totals });
          break;
        case "toast":
          dispatch({ type: "toast", text: msg.text });
          break;
        default:
          break;
      }
    },
    [dispatch],
  );
  useHostMessage(onHostMessage);

  const selected = useMemo(
    () => state.beats.find((b) => b.id === state.selectedBeat) ?? null,
    [state.beats, state.selectedBeat],
  );

  return (
    <div className="pulse-root">
      <div className="sidebar">
        <div className="sidebar-header">
          <span className="logo">⚡ Pulse</span>
          <button className="icon-btn" onClick={() => setTab(tab === "chat" ? "settings" : "chat")}>
            {tab === "chat" ? "Settings" : "Chat"}
          </button>
        </div>
        <BeatList
          beats={state.beats}
          selected={state.selectedBeat}
          running={state.running}
          onSelect={(beatId) => dispatch({ type: "select-beat", beatId })}
        />
      </div>
      <main className="main">
        {tab === "settings" ? (
          <SettingsPanel
            models={state.models}
            projects={state.projects}
            config={state.modelConfig}
            workflows={state.workflows}
            defaultWorkflow={state.defaultWorkflow}
          />
        ) : (
          <ChatPanel beat={selected} state={state} />
        )}
      </main>
      {state.toast !== null && <div className="toast">{state.toast}</div>}
    </div>
  );
}
