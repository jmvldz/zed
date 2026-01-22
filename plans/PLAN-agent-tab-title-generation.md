# Plan: Agent Tab Title Generation

## Goals
- Generate 2–3 word titles for agent tabs using an authenticated provider, not LM Studio by default.
- Ensure both native and external agent threads can generate titles on the Zed side.

## Steps
1. Inspect title-generation entry points for native threads (`crates/agent/src/thread.rs::generate_title`) and ACP threads (`crates/acp_thread/src/acp_thread.rs::generate_title`), plus where `summarization_model` is set (`crates/agent/src/agent.rs`) to confirm wiring and data flow.
2. Introduce a shared model-selection helper in `crates/language_model/src/registry.rs` (e.g., `thread_summary_fallback_model(&self, cx: &App) -> Option<ConfiguredModel>`) that:
   - Returns `thread_summary_model` only if its provider is authenticated.
   - Falls back to `default_fast_model`/`default_model` when authenticated.
   - Otherwise scans authenticated providers’ `default_fast_model`/`default_model` and picks the first available.
3. Wire the helper into native thread setup in `crates/agent/src/agent.rs` (replace `registry.thread_summary_model()` selection with the new helper) so `Thread::summarization_model` never binds to an unauthenticated provider; do the same in ACP title generation in `crates/acp_thread/src/acp_thread.rs::generate_title`.
4. Tighten the title prompt in `crates/agent_settings/src/prompts/summarize_thread_prompt.txt` to mandate 2–3 words, no punctuation; add a small normalization helper (e.g., `fn clamp_title_words(...)`) in `crates/agent/src/thread.rs` and `crates/acp_thread/src/acp_thread.rs` to enforce the word limit before calling `set_title`.
5. Update tests in `crates/agent/src/tests/mod.rs` (and add ACP tests if present) to verify:
   - Title generation uses fallback when the configured summary model is unauthenticated.
   - Generated titles are clamped to 2–3 words.
