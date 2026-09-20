import { apiKey } from "./config";
import { getSetting } from "./db";

export type Usage = { prompt_tokens: number; completion_tokens: number };
export type ToolCall = { id: string; name: string; arguments: string };
export type ChatResult = {
  content: string;
  tool_calls: ToolCall[];
  usage: Usage;
  finish_reason: string | null;
};
export type ChatMessage = Record<string, unknown>;

export type Model = {
  id: string;
  name: string;
  context_length: number | null;
  pricing: { prompt: string; completion: string };
};

export type Provider = {
  name: string;
  prefix: string;
  keySetting: string;
  chatSetup: (
    req: ChatRequest,
    key: string,
  ) => { url: string; headers: Record<string, string>; body: ChatMessage };
  models: () => Promise<Model[]>;
};

export type ChatRequest = {
  model: string;
  messages: ChatMessage[];
  temperature?: number;
  maxTokens?: number;
  jsonMode: boolean;
  tools?: ChatMessage[];
};

function num(v: unknown): number | null {
  const n = typeof v === "string" ? Number(v) : v;
  return typeof n === "number" && !Number.isNaN(n) ? n : null;
}

export function modelFromJson(v: Record<string, unknown>): Model {
  const id = (v.id as string) ?? "";
  const pricing = (v.pricing ?? {}) as Record<string, unknown>;
  let prompt = (pricing.prompt as string) ?? "";
  let completion = (pricing.completion as string) ?? "";
  if (!prompt) {
    const p = num(v.input_cost_per_token);
    if (p !== null) prompt = String(p);
  }
  if (!completion) {
    const c = num(v.output_cost_per_token);
    if (c !== null) completion = String(c);
  }
  const context = num(v.context_length) ?? num(v.max_input_tokens) ?? num(v.max_tokens) ?? null;
  const name = (v.name as string) || id;
  return { id, name, context_length: context, pricing: { prompt, completion } };
}

export function baseBody(req: ChatRequest): ChatMessage {
  const body: ChatMessage = { model: req.model, messages: req.messages };
  if (req.temperature !== undefined) body.temperature = req.temperature;
  if (req.maxTokens !== undefined) body.max_tokens = req.maxTokens;
  if (req.jsonMode) body.response_format = { type: "json_object" };
  if (req.tools) {
    body.tools = req.tools;
    body.tool_choice = "auto";
  }
  return body;
}

export async function fetchModelList(
  url: string,
  headers: Record<string, string>,
  name: string,
  prefix: string,
): Promise<Model[]> {
  const resp = await fetch(url, { headers });
  const text = await resp.text();
  if (!resp.ok) {
    throw new Error(`${name} request failed: HTTP ${resp.status}: ${text.slice(0, 2000)}`);
  }
  const parsed = JSON.parse(text) as { data?: Record<string, unknown>[] };
  if (!parsed.data) throw new Error(`Unexpected ${name} models response: ${text}`);
  return parsed.data.map((v) => {
    const m = modelFromJson(v);
    m.id = `${prefix}${m.id}`;
    return m;
  });
}

/* ---- providers ---- */

const OPENROUTER = "https://openrouter.ai/api/v1";
const OPENCODE = "https://opencode.ai/zen/go/v1";

function litellmBaseUrl(): string {
  const u = getSetting("litellm_base_url");
  if (u && u.trim()) return u.trim().replace(/\/+$/, "");
  return "http://localhost:4000/v1";
}

const providers: Provider[] = [
  {
    name: "OpenRouter",
    prefix: "OpenRouter - ",
    keySetting: "openrouter",
    chatSetup(req, key) {
      const body = baseBody(req);
      return {
        url: `${OPENROUTER}/chat/completions`,
        headers: {
          Authorization: `Bearer ${key}`,
          "HTTP-Referer": "https://pulse.dev",
          "X-Title": "Pulse",
        },
        body,
      };
    },
    async models() {
      const key = apiKey("openrouter") ?? "";
      return fetchModelList(
        `${OPENROUTER}/models`,
        { Authorization: `Bearer ${key}` },
        this.name,
        this.prefix,
      );
    },
  },
  {
    name: "OpenCode Go",
    prefix: "OpenCode - ",
    keySetting: "opencode",
    chatSetup(req, key) {
      return {
        url: `${OPENCODE}/chat/completions`,
        headers: {
          Authorization: `Bearer ${key}`,
          "User-Agent": "Pulse/1.0",
          "x-opencode-session": "pulse-default",
        },
        body: baseBody(req),
      };
    },
    async models() {
      return fetchModelList(
        `${OPENCODE}/models`,
        { "User-Agent": "Pulse/1.0" },
        this.name,
        this.prefix,
      );
    },
  },
  {
    name: "LiteLLM",
    prefix: "LiteLLM - ",
    keySetting: "litellm",
    chatSetup(req, key) {
      return {
        url: `${litellmBaseUrl()}/chat/completions`,
        headers: { Authorization: `Bearer ${key}` },
        body: baseBody(req),
      };
    },
    async models() {
      const key = apiKey("litellm") ?? "";
      return fetchModelList(
        `${litellmBaseUrl()}/models`,
        { Authorization: `Bearer ${key}` },
        this.name,
        this.prefix,
      );
    },
  },
];

export function providerFor(model: string): { provider: Provider; modelId: string } {
  for (const p of providers) {
    if (model.startsWith(p.prefix)) {
      return { provider: p, modelId: model.slice(p.prefix.length) };
    }
  }
  return { provider: providers[0], modelId: model };
}

/* ---- shared chat dispatch with one retry (drops json_mode) ---- */

export async function chatCompletion(
  model: string,
  messages: ChatMessage[],
  temperature: number | undefined,
  maxTokens: number | undefined,
  jsonMode: boolean,
  tools?: ChatMessage[],
): Promise<ChatResult> {
  const { provider: p, modelId } = providerFor(model);
  const key = apiKey(p.keySetting);
  if (!key) throw new Error(`No ${p.name} API key configured`);
  let req: ChatRequest = { model: modelId, messages, temperature, maxTokens, jsonMode, tools };
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      return await sendChat(p, req, key);
    } catch (e) {
      if (attempt === 0) {
        req = { ...req, jsonMode: false };
        await new Promise((r) => setTimeout(r, 400));
      } else {
        throw e;
      }
    }
  }
  throw new Error("unreachable");
}

async function sendChat(p: Provider, req: ChatRequest, key: string): Promise<ChatResult> {
  const { url, headers, body } = p.chatSetup(req, key);
  const resp = await fetch(url, {
    method: "POST",
    headers: { ...headers, "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const text = await resp.text();
  if (!resp.ok) {
    throw new Error(`${p.name} request failed: HTTP ${resp.status}: ${text.slice(0, 2000)}`);
  }
  const v = JSON.parse(text) as Record<string, any>;
  const message = v.choices?.[0]?.message;
  if (!message) throw new Error(`Unexpected ${p.name} response: ${text}`);
  const tool_calls: ToolCall[] = (message.tool_calls ?? [])
    .map((tc: any) => ({
      id: tc.id ?? "",
      name: tc.function?.name ?? "",
      arguments: tc.function?.arguments ?? "{}",
    }))
    .filter((tc: ToolCall) => tc.name);
  return {
    content: message.content ?? "",
    tool_calls,
    usage: {
      prompt_tokens: v.usage?.prompt_tokens ?? 0,
      completion_tokens: v.usage?.completion_tokens ?? 0,
    },
    finish_reason: v.choices?.[0]?.finish_reason ?? null,
  };
}

/* ---- streaming chat with SSE parsing ---- */

export async function chatCompletionStream(
  model: string,
  messages: ChatMessage[],
  temperature: number | undefined,
  maxTokens: number | undefined,
  jsonMode: boolean,
  tools: ChatMessage[] | undefined,
  onDelta: (t: string) => void,
): Promise<ChatResult> {
  const { provider: p, modelId } = providerFor(model);
  const key = apiKey(p.keySetting);
  if (!key) throw new Error(`No ${p.name} API key configured`);
  const req: ChatRequest = { model: modelId, messages, temperature, maxTokens, jsonMode, tools };
  let deltas = 0;
  const wrapped = (t: string) => {
    deltas++;
    onDelta(t);
  };
  const res = await sendChatStream(p, req, key, wrapped);
  if (!res.ok && deltas === 0) {
    return chatCompletion(model, messages, temperature, maxTokens, jsonMode, tools);
  }
  if (!res.ok) throw res.error;
  return res.value;
}

type StreamOutcome = { ok: true; value: ChatResult } | { ok: false; error: Error };

async function sendChatStream(
  p: Provider,
  req: ChatRequest,
  key: string,
  onDelta: (t: string) => void,
): Promise<StreamOutcome> {
  const { url, headers, body } = p.chatSetup(req, key);
  (body as Record<string, unknown>).stream = true;
  (body as Record<string, unknown>).stream_options = { include_usage: true };
  let resp: Response;
  try {
    resp = await fetch(url, {
      method: "POST",
      headers: { ...headers, "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch (e) {
    return { ok: false, error: new Error(`${p.name} request failed: ${e}`) };
  }
  if (!resp.ok || !resp.body) {
    const text = await resp.text();
    return {
      ok: false,
      error: new Error(`${p.name} request failed: HTTP ${resp.status}: ${text.slice(0, 2000)}`),
    };
  }
  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let content = "";
  const calls = new Map<number, ToolCall>();
  let usage: Usage = { prompt_tokens: 0, completion_tokens: 0 };
  let finish_reason: string | null = null;
  let buf = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });
    let nl: number;
    while ((nl = buf.indexOf("\n")) !== -1) {
      const line = buf.slice(0, nl).replace(/\r$/, "");
      buf = buf.slice(nl + 1);
      if (!line.startsWith("data:")) continue;
      const data = line.slice(5).trim();
      if (data === "[DONE]") continue;
      let v: Record<string, any>;
      try {
        v = JSON.parse(data);
      } catch {
        continue;
      }
      const delta = v.choices?.[0]?.delta;
      const text = delta?.content;
      if (typeof text === "string" && text) {
        content += text;
        onDelta(text);
      }
      for (const tc of delta?.tool_calls ?? []) {
        const e = calls.get(tc.index ?? 0) ?? { id: "", name: "", arguments: "" };
        e.id += tc.id ?? "";
        e.name += tc.function?.name ?? "";
        e.arguments += tc.function?.arguments ?? "";
        calls.set(tc.index ?? 0, e);
      }
      if (v.usage && typeof v.usage === "object") {
        usage = {
          prompt_tokens: v.usage.prompt_tokens ?? 0,
          completion_tokens: v.usage.completion_tokens ?? 0,
        };
      }
      const fr = v.choices?.[0]?.finish_reason;
      if (fr) finish_reason = fr;
    }
  }
  return {
    ok: true,
    value: { content, tool_calls: [...calls.values()], usage, finish_reason },
  };
}

/* ---- merged model list + 15-minute cache ---- */

const MODELS_TTL = 15 * 60 * 1000;
let modelsCache: { at: number; models: Model[] } | null = null;

async function fetchModels(): Promise<Model[]> {
  const models: Model[] = [];
  const errors: string[] = [];
  for (const p of providers) {
    try {
      models.push(...(await p.models()));
    } catch (e) {
      errors.push(`${p.name}: ${e}`);
    }
  }
  if (models.length === 0) throw new Error(errors.join("; "));
  models.sort((a, b) => a.id.localeCompare(b.id));
  modelsCache = { at: Date.now(), models };
  return models;
}

export async function listModels(): Promise<Model[]> {
  if (modelsCache && Date.now() - modelsCache.at < MODELS_TTL) return modelsCache.models;
  return fetchModels();
}

export function startModelsRefresh(): void {
  const tick = () => {
    fetchModels().catch((e) => console.error("models refresh failed:", e));
  };
  setInterval(tick, MODELS_TTL).unref?.();
}
