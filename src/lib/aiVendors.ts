// ── AI vendor preset registry (PRODUCT-SPEC §5.14.2) ───────────────
// Pure data. Two wire protocols cover every vendor here: Anthropic's
// Messages API and the OpenAI-compatible /chat/completions surface —
// which is what virtually the whole market serves (NVIDIA NIM,
// Xiaomi MiMo, DeepSeek, Kimi, GLM, DashScope, Ark, SiliconFlow,
// OpenRouter, Groq, local Ollama/LM Studio/vLLM, …).
//
// Base URLs are DEFAULTS, not contracts: the settings UI always lets
// the user edit them, so a vendor moving its endpoint never blocks
// anyone. Each vendor gets its own keyring slot (`pier-x.ai.<id>`),
// so switching vendors never reuses another vendor's key.

import type { AiProviderKind } from "./ai";

export type AiVendorGroup = "official" | "cn" | "intl" | "local" | "agent-cli" | "custom";

export type AiVendorPreset = {
  /** Stable id — keyring slot suffix and persisted selection. */
  id: string;
  /** Display label (left untranslated — these are brand names). */
  label: string;
  group: AiVendorGroup;
  /** Wire protocol the endpoint speaks. */
  kind: AiProviderKind;
  /** Default endpoint base; "" = the user must fill it in. */
  baseUrl: string;
  /** Whether the endpoint normally requires an API key. */
  needsKey: boolean;
  /** Key format hint shown as the input placeholder. */
  keyHint?: string;
  /** Example model id for the model-input placeholder. */
  modelHint?: string;
  /** `kind === "cli"` only: which agent CLI this preset drives
   *  ("claude-code" / "codex"); see PRODUCT-SPEC §5.14.8. */
  cliFlavor?: string;
};

export const AI_VENDOR_GROUP_LABELS: Record<AiVendorGroup, string> = {
  official: "Model vendors",
  cn: "China platforms",
  intl: "International platforms",
  local: "Local inference",
  "agent-cli": "Local agent CLI",
  custom: "Custom",
};

export const AI_VENDORS: AiVendorPreset[] = [
  // ── Model vendors ────────────────────────────────────────────
  {
    id: "anthropic",
    label: "Anthropic",
    group: "official",
    kind: "anthropic",
    baseUrl: "https://api.anthropic.com",
    needsKey: true,
    keyHint: "sk-ant-…",
    modelHint: "claude-opus-4-8",
  },
  {
    id: "openai",
    label: "OpenAI",
    group: "official",
    kind: "openai",
    baseUrl: "https://api.openai.com/v1",
    needsKey: true,
    keyHint: "sk-…",
    modelHint: "gpt-5.5",
  },
  // ── China platforms ──────────────────────────────────────────
  {
    id: "deepseek",
    label: "DeepSeek",
    group: "cn",
    kind: "openai",
    baseUrl: "https://api.deepseek.com/v1",
    needsKey: true,
    modelHint: "deepseek-chat",
  },
  {
    id: "moonshot",
    label: "Kimi (Moonshot)",
    group: "cn",
    kind: "openai",
    baseUrl: "https://api.moonshot.cn/v1",
    needsKey: true,
  },
  {
    id: "zhipu",
    label: "智谱 GLM",
    group: "cn",
    kind: "openai",
    baseUrl: "https://open.bigmodel.cn/api/paas/v4",
    needsKey: true,
  },
  {
    id: "dashscope",
    label: "通义 Qwen (DashScope)",
    group: "cn",
    kind: "openai",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    needsKey: true,
    modelHint: "qwen-plus",
  },
  {
    id: "ark",
    label: "豆包 (火山方舟)",
    group: "cn",
    kind: "openai",
    baseUrl: "https://ark.cn-beijing.volces.com/api/v3",
    needsKey: true,
  },
  {
    id: "hunyuan",
    label: "腾讯混元",
    group: "cn",
    kind: "openai",
    baseUrl: "https://api.hunyuan.cloud.tencent.com/v1",
    needsKey: true,
  },
  {
    id: "qianfan",
    label: "百度千帆",
    group: "cn",
    kind: "openai",
    baseUrl: "https://qianfan.baidubce.com/v2",
    needsKey: true,
  },
  {
    id: "minimax",
    label: "MiniMax",
    group: "cn",
    kind: "openai",
    baseUrl: "https://api.minimaxi.com/v1",
    needsKey: true,
  },
  {
    id: "xiaomi",
    label: "小米 MiMo",
    group: "cn",
    kind: "openai",
    baseUrl: "https://api.xiaomimimo.com/v1",
    needsKey: true,
    modelHint: "mimo-v2.5-pro",
  },
  {
    id: "siliconflow",
    label: "SiliconFlow (硅基流动)",
    group: "cn",
    kind: "openai",
    baseUrl: "https://api.siliconflow.cn/v1",
    needsKey: true,
  },
  // ── International platforms ──────────────────────────────────
  {
    id: "nvidia",
    label: "NVIDIA NIM",
    group: "intl",
    kind: "openai",
    baseUrl: "https://integrate.api.nvidia.com/v1",
    needsKey: true,
    keyHint: "nvapi-…",
  },
  {
    id: "openrouter",
    label: "OpenRouter",
    group: "intl",
    kind: "openai",
    baseUrl: "https://openrouter.ai/api/v1",
    needsKey: true,
  },
  {
    id: "groq",
    label: "Groq",
    group: "intl",
    kind: "openai",
    baseUrl: "https://api.groq.com/openai/v1",
    needsKey: true,
  },
  {
    id: "mistral",
    label: "Mistral",
    group: "intl",
    kind: "openai",
    baseUrl: "https://api.mistral.ai/v1",
    needsKey: true,
  },
  {
    id: "xai",
    label: "xAI Grok",
    group: "intl",
    kind: "openai",
    baseUrl: "https://api.x.ai/v1",
    needsKey: true,
  },
  {
    id: "gemini",
    label: "Google Gemini",
    group: "intl",
    kind: "openai",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    needsKey: true,
  },
  // ── Local inference ──────────────────────────────────────────
  {
    id: "ollama",
    label: "Ollama",
    group: "local",
    kind: "ollama",
    baseUrl: "http://localhost:11434/v1",
    needsKey: false,
  },
  {
    id: "lmstudio",
    label: "LM Studio",
    group: "local",
    kind: "openai",
    baseUrl: "http://localhost:1234/v1",
    needsKey: false,
  },
  {
    id: "vllm",
    label: "vLLM",
    group: "local",
    kind: "openai",
    baseUrl: "http://localhost:8000/v1",
    needsKey: false,
  },
  // ── Local agent CLI (reuse your logged-in CLI, no API key; §5.14.8) ──
  {
    id: "claude-code",
    label: "Claude Code",
    group: "agent-cli",
    kind: "cli",
    baseUrl: "",
    needsKey: false,
    cliFlavor: "claude-code",
    modelHint: "sonnet / claude-sonnet-4-6",
  },
  {
    id: "codex",
    label: "OpenAI Codex",
    group: "agent-cli",
    kind: "cli",
    baseUrl: "",
    needsKey: false,
    cliFlavor: "codex",
    modelHint: "gpt-5.5",
  },
  // ── Custom ───────────────────────────────────────────────────
  {
    id: "custom",
    label: "Custom (OpenAI-compatible)",
    group: "custom",
    kind: "openai",
    baseUrl: "",
    needsKey: true,
  },
  {
    id: "custom-anthropic",
    label: "Custom (Anthropic-compatible)",
    group: "custom",
    kind: "anthropic",
    baseUrl: "",
    needsKey: true,
  },
];

const FALLBACK = AI_VENDORS.find((v) => v.id === "custom")!;

export function aiVendorById(id: string): AiVendorPreset {
  return AI_VENDORS.find((v) => v.id === id) ?? FALLBACK;
}

/** Vendors in stable optgroup order for the settings <select>. */
export function aiVendorsByGroup(): { group: AiVendorGroup; vendors: AiVendorPreset[] }[] {
  const order: AiVendorGroup[] = ["official", "cn", "intl", "local", "agent-cli", "custom"];
  return order.map((group) => ({
    group,
    vendors: AI_VENDORS.filter((v) => v.group === group),
  }));
}
