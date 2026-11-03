//! The speech models kept on disk.

/// The weight files the engine can run, in order of preference.
pub const WEIGHT_FILES: [&str; 3] = ["model.q8.gguf", "model.q4k.gguf", "model.safetensors"];

/// The small files a model folder needs besides its weights.
pub const SMALL_FILES: [&str; 9] = [
    "config.json",
    "generation_config.json",
    "processor_config.json",
    "tokenizer.json",
    "tokenizer.5.0.0.json",
    "tokenizer.model",
    "tokenizer_config.json",
    "chat_template.jinja",
    "default-voice.safetensors",
];
