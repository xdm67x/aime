import { getModelConfig, type ModelConfig } from "./config";
import { appendMessages, type BeatEntry } from "./db";
import * as beats from "./beats";
import * as projects from "./projects";
import {
  chatCompletion,
  chatCompletionStream,
  listModels,
  type ChatMessage,
  type Usage,
} from "./providers";
import { discover, type SkillInfo } from "./skills";
import { execute, stripDiff, toolDefinitions, type ToolStep } from "./tools";

/* ---- prompts (mirrors of pulse-core/src/prompts/*.md) ---- */

const CLASSIFIER_PROMPT = `You route a user task to one of three model tiers by difficulty.
- "high": deep reasoning, multi-step planning, architecture, hard debugging, or high-stakes problems that benefit from reflection.
- "base": typical implementation work — writing or modifying code, explaining, straightforward but non-trivial tasks.
- "low": simple, basic tasks — quick facts, greetings, formatting, trivial questions.
Reply with only JSON: {"tier":"high"|"base"|"low"}.`;

const AGENT_NOTE = `Tools are available (read_file, write_file, edit_file, grep, bash, and skill_* loaders). Use them when they help: read before editing, search before assuming, verify by running. skill_* tools load the full instructions of a specialized skill on demand. When the work is done, reply with the final answer as plain text (no tool call).`;

const SUMMARIZE_PROMPT = `Summarize this conversation so far into a compact context brief. Keep facts, decisions and open questions.
{{history}}
Final answer:`;

const REFLEXION_PROMPT = `Task:{{prompt}}
Draft answer:{{draft}}
Tool evidence:{{evidence}}
Critique the draft for correctness, gaps and clarity, then give the final, improved answer.`;

const SYSTEM_PROMPT = `# Session instructions
- Keep replies concise.`;

export function fill(template: string, vars: Record<string, string>): string {
  let out = template;
  for (const [k, v] of Object.entries(vars)) {
    out = out.replaceAll(`{{${k}}}`, v);
  }
  return out;
}

/* ---- live events ---- */

export type TaskEvent =
  | { type: "start"; model: string; tier: string }
  | { type: "delta"; text: string }
  | { type: "tool"; tool: string; arguments: string; result: string; error: boolean }
  | { type: "step"; text: string };

export type TaggedEvent = { beatId: number } & TaskEvent;

export type OnEvent = (ev: TaggedEvent) => void;

/* ---- per-beat cancellation ---- */

const cancelled = new Set<number>();

export function cancelCurrent(beatId: number): void {
  cancelled.add(beatId);
}

function isCancelled(beatId: number): boolean {
  return cancelled.has(beatId);
}

export const STOPPED = "stopped";

/* ---- tiers ---- */

export type Tier = "high" | "base" | "low";

function parseTier(s: string): Tier | null {
  const t = s.trim().toLowerCase();
  return t === "high" || t === "base" || t === "low" ? t : null;
}

function extractJson(text: string): Record<string, unknown> | null {
  const start = text.indexOf("{");
  const end = text.lastIndexOf("}");
  if (start === -1 || end <= start) return null;
  try {
    return JSON.parse(text.slice(start, end + 1)) as Record<string, unknown>;
  } catch {
    return null;
  }
}

export function extractTier(reply: string): Tier {
  const v = extractJson(reply);
  const tier = v?.tier;
  return typeof tier === "string" ? (parseTier(tier) ?? "base") : "base";
}

async function classify(classifier: string, prompt: string): Promise<Tier> {
  const r = await chatCompletion(
    classifier,
    [
      { role: "system", content: CLASSIFIER_PROMPT },
      { role: "user", content: prompt },
    ],
    0.0,
    64,
    true,
  );
  return extractTier(r.content);
}

/* ---- context + session prompt ---- */

async function summarizeHistory(beatId: number, model: string): Promise<string> {
  const prior = beats.getBeatMessages(beatId);
  const text = prior
    .filter((m) => m.role && typeof m.content === "string")
    .map((m) => `${m.role}: ${m.content}`)
    .join("\n");
  if (!text) return "";
  const tail = text.slice(-6000);
  const r = await chatCompletion(
    model,
    [
      {
        role: "user",
        content: fill(SUMMARIZE_PROMPT, { history: tail }),
      },
    ],
    0.2,
    512,
    false,
  );
  return `Conversation context so far:\n${r.content}`;
}

function priorTurns(beatId: number): ChatMessage[] {
  return beats
    .getBeatMessages(beatId)
    .filter(
      (m) =>
        (m.role === "user" || m.role === "assistant") &&
        typeof m.content === "string" &&
        m.content.trim() !== "",
    )
    .map((m) => replayMessage(m));
}

function replayMessage(m: BeatEntry): ChatMessage {
  const text = typeof m.content === "string" ? m.content : "";
  if (m.images && m.images.length > 0) {
    const parts: ChatMessage[] = m.images.map((u) => ({
      type: "image_url",
      image_url: { url: u },
    }));
    if (text.trim()) parts.push({ type: "text", text });
    return { role: m.role, content: parts };
  }
  return { role: m.role, content: text };
}

function userMessage(prompt: string, images: string[]): ChatMessage {
  if (!images.length) return { role: "user", content: prompt };
  const parts: ChatMessage[] = images.map((u) => ({
    type: "image_url",
    image_url: { url: u },
  }));
  parts.push({ type: "text", text: prompt });
  return { role: "user", content: parts };
}

function userEntry(prompt: string, images: string[]): BeatEntry {
  if (!images.length) return { role: "user", content: prompt };
  return { role: "user", content: prompt, images };
}

function systemMessage(brief: string, session: string, note: string): ChatMessage {
  let content = "";
  if (brief) content += `${brief}\n\n`;
  if (session) content += `Session instructions:\n${session}\n\n`;
  content += note;
  return { role: "system", content };
}

/* ---- agentic loop ---- */

const CONTEXT_LIMIT_PERCENT = 90;

async function contextLimitReached(modelId: string, promptTokens: number): Promise<boolean> {
  const models = await listModels().catch(() => []);
  const len = models.find((m) => m.id === modelId)?.context_length;
  if (!len || len === 0) return false;
  return (promptTokens / len) * 100 >= CONTEXT_LIMIT_PERCENT;
}

async function usageCost(modelId: string, prompt: number, completion: number): Promise<number> {
  const models = await listModels().catch(() => []);
  const m = models.find((x) => x.id === modelId);
  if (!m) return 0;
  return Number(m.pricing.prompt || 0) * prompt + Number(m.pricing.completion || 0) * completion;
}

type ModelUsage = {
  model: string;
  prompt_tokens: number;
  completion_tokens: number;
  cost_usd: number;
};

async function recordUsage(beatId: number, mu: ModelUsage, u: Usage): Promise<void> {
  mu.prompt_tokens += u.prompt_tokens;
  mu.completion_tokens += u.completion_tokens;
  mu.cost_usd += await usageCost(mu.model, u.prompt_tokens, u.completion_tokens);
  try {
    beats.recordUsage(beatId, mu.model, u.prompt_tokens, u.completion_tokens, mu.cost_usd);
  } catch (e) {
    console.error("usage record failed:", e);
  }
}

async function agenticLoop(
  model: string,
  beatId: number,
  onEvent: (ev: TaskEvent) => void,
  messages: ChatMessage[],
  entries: BeatEntry[],
  tools: ChatMessage[],
  cwd: string | null,
): Promise<{ answer: string; steps: ToolStep[]; usage: Usage; contextFull: boolean }> {
  const steps: ToolStep[] = [];
  const usage: Usage = { prompt_tokens: 0, completion_tokens: 0 };
  let nudged = false;
  for (;;) {
    if (isCancelled(beatId)) throw new Error(STOPPED);
    const r = await chatCompletionStream(model, messages, 0.7, undefined, false, tools, (t) =>
      onEvent({ type: "delta", text: t }),
    );
    usage.prompt_tokens += r.usage.prompt_tokens;
    usage.completion_tokens += r.usage.completion_tokens;
    const lastPromptTokens = r.usage.prompt_tokens;

    const done = r.tool_calls.find((tc) => tc.name === "task_complete");
    if (done) {
      let summary = "";
      try {
        const v = JSON.parse(done.arguments) as { summary?: unknown };
        if (typeof v.summary === "string" && v.summary.trim()) summary = v.summary;
      } catch {
        /* malformed — fall through */
      }
      if (!summary) {
        persistRound(model, r, messages, entries, onEvent);
        for (const tc of r.tool_calls) {
          messages.push({
            role: "tool",
            tool_call_id: tc.id,
            content:
              "task_complete requires a non-empty 'summary' argument. Call it again with your full final answer.",
          });
        }
        continue;
      }
      onEvent({ type: "step", text: summary });
      entries.push({ role: "assistant", model, content: summary });
      return { answer: summary, steps, usage, contextFull: false };
    }

    if (r.tool_calls.length === 0) {
      const blank = r.content.trim() === "";
      if (!blank && r.finish_reason !== "length" && r.finish_reason !== "tool_calls") {
        const dup =
          entries.length > 0 &&
          entries[entries.length - 1].role === "assistant" &&
          entries[entries.length - 1].content === r.content;
        if (!dup) {
          onEvent({ type: "step", text: r.content });
          entries.push({ role: "assistant", model, content: r.content });
          messages.push({ role: "assistant", content: r.content });
        }
        messages.push({
          role: "user",
          content:
            "Your reply arrived without a tool call, so the task is still open. Continue working with tools, or call `task_complete` with your final answer if everything is done.",
        });
        continue;
      }
      if (!blank) {
        onEvent({ type: "step", text: r.content });
        entries.push({ role: "assistant", model, content: r.content });
        messages.push({ role: "assistant", content: r.content });
      } else if (!nudged) {
        nudged = true;
        messages.push({ role: "user", content: "Continue." });
      }
      continue;
    }

    if (r.content.trim()) {
      onEvent({ type: "step", text: r.content });
      entries.push({ role: "assistant", model, content: r.content });
    }
    messages.push({
      role: "assistant",
      content: r.content,
      tool_calls: r.tool_calls.map((tc) => ({
        id: tc.id,
        type: "function",
        function: { name: tc.name, arguments: tc.arguments },
      })),
    });
    for (const tc of r.tool_calls) {
      if (isCancelled(beatId)) throw new Error(STOPPED);
      let output: string;
      let error = false;
      try {
        output = execute(tc.name, tc.arguments, cwd);
      } catch (e) {
        output = String(e);
        error = true;
      }
      const modelOutput = stripDiff(output);
      onEvent({ type: "tool", tool: tc.name, arguments: tc.arguments, result: output, error });
      entries.push({
        role: "tool",
        model: tc.name,
        arguments: tc.arguments,
        content: modelOutput,
        error,
        raw_content: output,
      });
      steps.push({ tool: tc.name, arguments: tc.arguments, result: output, error });
      messages.push({ role: "tool", tool_call_id: tc.id, content: modelOutput });
    }
    if (await contextLimitReached(model, lastPromptTokens)) {
      console.error("agentic loop hit the session context limit; forcing a final answer");
      const r2 = await chatCompletionStream(
        model,
        messages,
        0.7,
        undefined,
        false,
        undefined,
        (t) => onEvent({ type: "delta", text: t }),
      );
      usage.prompt_tokens += r2.usage.prompt_tokens;
      usage.completion_tokens += r2.usage.completion_tokens;
      return { answer: r2.content, steps, usage, contextFull: true };
    }
  }
}

function persistRound(
  model: string,
  r: { content: string; tool_calls: { id: string; name: string; arguments: string }[] },
  messages: ChatMessage[],
  entries: BeatEntry[],
  onEvent: (ev: TaskEvent) => void,
): void {
  if (r.content.trim()) {
    onEvent({ type: "step", text: r.content });
    entries.push({ role: "assistant", model, content: r.content });
  }
  messages.push({
    role: "assistant",
    content: r.content,
    tool_calls: r.tool_calls.map((tc) => ({
      id: tc.id,
      type: "function",
      function: { name: tc.name, arguments: tc.arguments },
    })),
  });
}

/* ---- reflexion (high tier) ---- */

async function reflexion(
  model: string,
  onEvent: (ev: TaskEvent) => void,
  prompt: string,
  draft: string,
  toolSteps: ToolStep[],
  brief: string,
  session: string,
): Promise<{ content: string; usage: Usage }> {
  const evidence =
    toolSteps.length === 0
      ? "none (no tools used)"
      : toolSteps
          .map((s) => `- ${s.tool}(${s.arguments}) → ${s.error ? "FAILED" : "ok"}`)
          .join("\n");
  return chatCompletionStream(
    model,
    [
      systemMessage(brief, session, ""),
      {
        role: "user",
        content: fill(REFLEXION_PROMPT, { prompt, draft, evidence }),
      },
    ],
    0.5,
    undefined,
    false,
    undefined,
    (t) => onEvent({ type: "delta", text: t }),
  );
}

/* ---- task runner ---- */

export type TaskResult = {
  tier: string;
  model: string;
  steps: string[];
  tool_steps: ToolStep[];
  answer: string;
  usage: ModelUsage[];
  cost_usd: number;
  context_percent: number | null;
  context_full: boolean;
  new_beat_id: number | null;
};

async function compactSession(
  beatId: number,
  onEvent: (ev: TaskEvent) => void,
): Promise<TaskResult> {
  const cfg = getModelConfig();
  const classifier = cfg.classifier.trim();
  if (!classifier) {
    throw new Error("No classifier model configured — set the four models in Settings.");
  }
  const summary = await summarizeHistory(beatId, classifier);
  if (!summary.trim()) {
    throw new Error("Nothing to compact — this session has no messages yet.");
  }
  const newBeat = beats.createSummaryBeat(beatId, summary);
  beats.setBeatArchived(beatId, true);
  beats.setContextFull(beatId, false);
  onEvent({
    type: "step",
    text: `Session compacted into a new session: “${newBeat.name}”.\n\n${summary}`,
  });
  return {
    tier: "low",
    model: classifier,
    steps: [],
    tool_steps: [],
    answer: `Compacted. New session “${newBeat.name}” holds the summary:\n\n${summary}`,
    usage: [],
    cost_usd: 0,
    context_percent: null,
    context_full: false,
    new_beat_id: newBeat.id,
  };
}

export async function runTask(
  beatId: number,
  prompt: string,
  images: string[],
  onEvent: OnEvent,
): Promise<TaskResult> {
  cancelled.delete(beatId);
  const emit = (ev: TaskEvent) => onEvent({ beatId, ...ev });
  return runTaskInner(beatId, prompt, images, emit);
}

async function runTaskInner(
  beatId: number,
  prompt: string,
  images: string[],
  onEvent: (ev: TaskEvent) => void,
): Promise<TaskResult> {
  if (beats.isContextFull(beatId) && prompt.trim().toLowerCase() !== "/compact") {
    throw new Error(
      "Session context limit reached. Run /compact to open a new session holding only a summary of this one.",
    );
  }
  if (prompt.trim().toLowerCase() === "/compact") {
    return compactSession(beatId, onEvent);
  }
  const cfg: ModelConfig = getModelConfig();
  const classifier = cfg.classifier.trim();
  if (!classifier) {
    throw new Error("No classifier model configured — set the four models in Settings.");
  }
  const session = SYSTEM_PROMPT;
  const brief = await summarizeHistory(beatId, classifier);
  const tier = await classify(classifier, prompt);
  const model = (tier === "high" ? cfg.high : tier === "base" ? cfg.base : cfg.low).trim();
  if (!model) {
    throw new Error(`No ${tier} model configured — set the four models in Settings.`);
  }
  const mainUsage: ModelUsage = { model, prompt_tokens: 0, completion_tokens: 0, cost_usd: 0 };
  onEvent({ type: "start", model, tier });

  const discovered: SkillInfo[] = discover();
  const toolDefs = toolDefinitions(discovered);
  const entries: BeatEntry[] = [userEntry(prompt, images)];

  let answer: string;
  let steps: string[] = [];
  let toolSteps: ToolStep[] = [];
  let ctxFull = false;

  if (tier === "high" || tier === "base") {
    const wd = projects.workingDir(beatId);
    let note = AGENT_NOTE;
    if (wd) {
      note += `\n\nWorking directory: ${wd}. Relative tool paths resolve against it, bash runs inside it.\n\n`;
      const agents = projects.agentsNote(wd);
      if (agents) note += agents;
    }
    const msgs: ChatMessage[] = [
      systemMessage(brief, session, note),
      ...priorTurns(beatId),
      userMessage(prompt, images),
    ];
    const loop = await agenticLoop(model, beatId, onEvent, msgs, entries, toolDefs, wd);
    if (tier === "high") {
      onEvent({ type: "step", text: loop.answer });
      entries.push({ role: "assistant", model, content: loop.answer });
      const ref = await reflexion(model, onEvent, prompt, loop.answer, loop.steps, brief, session);
      const combined: Usage = {
        prompt_tokens: loop.usage.prompt_tokens + ref.usage.prompt_tokens,
        completion_tokens: loop.usage.completion_tokens + ref.usage.completion_tokens,
      };
      await recordUsage(beatId, mainUsage, combined);
      answer = ref.content;
      steps = [loop.answer];
      toolSteps = loop.steps;
      ctxFull = loop.contextFull;
    } else {
      await recordUsage(beatId, mainUsage, loop.usage);
      answer = loop.answer;
      toolSteps = loop.steps;
      ctxFull = loop.contextFull;
    }
  } else {
    const msgs: ChatMessage[] = [
      systemMessage(brief, session, ""),
      ...priorTurns(beatId),
      userMessage(prompt, images),
    ];
    const r = await chatCompletionStream(model, msgs, 0.7, undefined, false, undefined, (t) =>
      onEvent({ type: "delta", text: t }),
    );
    await recordUsage(beatId, mainUsage, r.usage);
    answer = r.content;
  }

  const costUsd = mainUsage.cost_usd;
  const modelsList = await listModels().catch(() => []);
  const ctxLen = modelsList.find((m) => m.id === model)?.context_length;
  const contextPercent =
    ctxLen && ctxLen > 0 ? Math.min(100, (mainUsage.prompt_tokens / ctxLen) * 100) : null;
  const contextFull =
    ctxFull || (contextPercent !== null && contextPercent >= CONTEXT_LIMIT_PERCENT);
  if (contextFull) beats.setContextFull(beatId, true);

  if (!answer.trim()) {
    answer =
      [...entries].reverse().find((e) => e.role === "assistant" && e.content.trim())?.content ?? "";
  }
  const dup =
    entries.length > 0 &&
    entries[entries.length - 1].role === "assistant" &&
    entries[entries.length - 1].content === answer;
  if (!dup) entries.push({ role: "assistant", model, content: answer });
  appendMessages(beatId, entries);
  return {
    tier,
    model,
    steps,
    tool_steps: toolSteps,
    answer,
    usage: [mainUsage],
    cost_usd: costUsd,
    context_percent: contextPercent,
    context_full: contextFull,
    new_beat_id: null,
  };
}
