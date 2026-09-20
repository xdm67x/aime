import { getSetting, setSetting } from "./db";

const PROVIDERS = ["openrouter", "opencode", "litellm"] as const;
export type ProviderName = (typeof PROVIDERS)[number];

export function apiKey(provider: string): string | null {
  return getSetting(`${provider}_api_key`);
}

export function saveApiKey(provider: string, key: string): void {
  if (!PROVIDERS.includes(provider as ProviderName)) {
    throw new Error(`Unknown provider: ${provider}`);
  }
  setSetting(`${provider}_api_key`, key.trim());
}

export function baseUrl(provider: string): string | null {
  return getSetting(`${provider}_base_url`);
}

export function saveBaseUrl(provider: string, url: string): void {
  if (provider !== "litellm") {
    throw new Error(`Unknown provider: ${provider}`);
  }
  setSetting(`${provider}_base_url`, url.trim());
}

export type ModelConfig = {
  classifier: string;
  high: string;
  base: string;
  low: string;
};

const KEYS: { [K in keyof ModelConfig]: string } = {
  classifier: "model_classifier",
  high: "model_high",
  base: "model_base",
  low: "model_low",
};

export function getModelConfig(): ModelConfig {
  return {
    classifier: getSetting(KEYS.classifier) ?? "",
    high: getSetting(KEYS.high) ?? "",
    base: getSetting(KEYS.base) ?? "",
    low: getSetting(KEYS.low) ?? "",
  };
}

export function saveModelConfig(config: ModelConfig): void {
  for (const k of Object.keys(KEYS) as (keyof ModelConfig)[]) {
    setSetting(KEYS[k], config[k].trim());
  }
}
