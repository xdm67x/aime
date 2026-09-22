import { useCallback, useMemo, useState } from "react";
import type {
  Beat,
  Model,
  ModelConfig,
  Project,
  TaskEvent,
  TaskResult,
  UsageTotal,
} from "../protocol";

export type LiveTurn = {
  streaming: string;
  events: (TaskEvent & { key: number })[];
};

export type PulseState = {
  beats: Beat[];
  selectedBeat: number | null;
  messages: Record<number, unknown[]>;
  projects: Project[];
  models: Model[];
  modelConfig: ModelConfig | null;
  live: Record<number, LiveTurn>;
  running: Record<number, boolean>;
  usage: Record<number, UsageTotal[]>;
  toast: string | null;
  results: Record<number, TaskResult[]>;
  error: Record<number, string | null>;
};

export const initialState: PulseState = {
  beats: [],
  selectedBeat: null,
  messages: {},
  projects: [],
  models: [],
  modelConfig: null,
  live: {},
  running: {},
  usage: {},
  toast: null,
  results: {},
  error: {},
};

export type Action =
  | { type: "beats"; beats: Beat[] }
  | { type: "select-beat"; beatId: number }
  | { type: "beat-messages"; beatId: number; messages: unknown[] }
  | { type: "projects"; projects: Project[] }
  | { type: "models"; models: Model[] }
  | { type: "model-config"; config: ModelConfig }
  | { type: "task-event"; beatId: number; ev: TaskEvent }
  | { type: "task-result"; beatId: number; result: TaskResult }
  | { type: "task-error"; beatId: number; error: string }
  | { type: "usage-totals"; beatId: number; totals: UsageTotal[] }
  | { type: "toast"; text: string | null }
  | { type: "clear-error"; beatId: number };

let eventKey = 0;

export function reducer(state: PulseState, action: Action): PulseState {
  switch (action.type) {
    case "beats": {
      const selected =
        state.selectedBeat !== null && action.beats.some((b) => b.id === state.selectedBeat)
          ? state.selectedBeat
          : (action.beats.find((b) => !b.archived)?.id ?? null);
      return { ...state, beats: action.beats, selectedBeat: selected };
    }
    case "select-beat":
      return { ...state, selectedBeat: action.beatId };
    case "beat-messages":
      return {
        ...state,
        messages: { ...state.messages, [action.beatId]: action.messages },
      };
    case "projects":
      return { ...state, projects: action.projects };
    case "models":
      return { ...state, models: action.models };
    case "model-config":
      return { ...state, modelConfig: action.config };
    case "task-event": {
      const turn = state.live[action.beatId] ?? { streaming: "", events: [] };
      const events = [...turn.events, { ...action.ev, key: eventKey++ }];
      const streaming =
        action.ev.type === "delta" ? turn.streaming + action.ev.text : turn.streaming;
      const live = { ...state.live, [action.beatId]: { streaming, events } };
      const running = { ...state.running, [action.beatId]: true };
      const error = { ...state.error, [action.beatId]: null };
      return { ...state, live, running, error };
    }
    case "task-result": {
      const prev = state.results[action.beatId] ?? [];
      return {
        ...state,
        results: { ...state.results, [action.beatId]: [...prev, action.result] },
        live: { ...state.live, [action.beatId]: { streaming: "", events: [] } },
        running: { ...state.running, [action.beatId]: false },
      };
    }
    case "task-error":
      return {
        ...state,
        running: { ...state.running, [action.beatId]: false },
        error: { ...state.error, [action.beatId]: action.error },
      };
    case "usage-totals":
      return { ...state, usage: { ...state.usage, [action.beatId]: action.totals } };
    case "toast":
      return { ...state, toast: action.text };
    case "clear-error":
      return { ...state, error: { ...state.error, [action.beatId]: null } };
  }
}

export function usePulseStore() {
  const [state, setState] = useState<PulseState>(initialState);
  const dispatch = useCallback((action: Action) => {
    setState((s) => reducer(s, action));
  }, []);
  return useMemo(() => ({ state, dispatch }), [state, dispatch]);
}
