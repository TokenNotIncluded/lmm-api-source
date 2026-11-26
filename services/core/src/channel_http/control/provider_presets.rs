//! Provider identity is display metadata. Protocol and endpoint govern execution.
use crate::routing::Protocol;
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct ProviderPreset {
    pub id: &'static str,
    label: &'static str,
    icon: &'static str,
    protocol: Protocol,
    endpoint: &'static str,
}

const fn preset(
    id: &'static str,
    label: &'static str,
    icon: &'static str,
    protocol: Protocol,
    endpoint: &'static str,
) -> ProviderPreset {
    ProviderPreset {
        id,
        label,
        icon,
        protocol,
        endpoint,
    }
}

use Protocol::*;

pub(super) const PROVIDERS: &[ProviderPreset] = &[
    preset(
        "openai",
        "OpenAI",
        "OpenAI",
        OpenAiChat,
        "https://api.openai.com/v1",
    ),
    preset(
        "anthropic",
        "Anthropic",
        "Anthropic",
        AnthropicMessages,
        "https://api.anthropic.com/v1",
    ),
    preset(
        "gemini",
        "Google Gemini",
        "Gemini",
        Gemini,
        "https://generativelanguage.googleapis.com/v1beta",
    ),
    preset(
        "deepseek",
        "DeepSeek",
        "DeepSeek",
        OpenAiChat,
        "https://api.deepseek.com",
    ),
    preset(
        "openrouter",
        "OpenRouter",
        "OpenRouter",
        OpenAiChat,
        "https://openrouter.ai/api/v1",
    ),
    preset(
        "groq",
        "Groq",
        "Groq",
        OpenAiChat,
        "https://api.groq.com/openai/v1",
    ),
    preset(
        "mistral",
        "Mistral",
        "Mistral",
        OpenAiChat,
        "https://api.mistral.ai/openai/v1",
    ),
    preset("xai", "xAI", "XAI", OpenAiChat, "https://api.x.ai/v1"),
    preset(
        "together",
        "Together AI",
        "Together",
        OpenAiChat,
        "https://api.together.xyz/v1",
    ),
    preset(
        "fireworks",
        "Fireworks AI",
        "Fireworks",
        OpenAiChat,
        "https://api.fireworks.ai/inference/v1",
    ),
    preset(
        "perplexity",
        "Perplexity",
        "Perplexity",
        OpenAiChat,
        "https://api.perplexity.ai",
    ),
    preset(
        "jina",
        "Jina AI",
        "Jina",
        JinaRerank,
        "https://api.jina.ai/v1",
    ),
    preset("aws_bedrock", "AWS Bedrock", "Aws", OpenAiChat, ""),
    preset("azure_openai", "Azure OpenAI", "Azure", OpenAiChat, ""),
    preset("system_one", "System One", "", SystemOne, ""),
    preset("custom", "Custom", "", OpenAiChat, ""),
];
