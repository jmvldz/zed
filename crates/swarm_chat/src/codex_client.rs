use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{anyhow, Result};
use futures::channel::mpsc;
use futures::SinkExt;
use gpui::{BackgroundExecutor, Task};
use serde::{Deserialize, Serialize};
use smol::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use smol::prelude::*;
use smol::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexConfig {
    pub cli_path: String,
    pub repo_root: PathBuf,
    pub add_dirs: Vec<PathBuf>,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            cli_path: "codex".to_string(),
            repo_root: PathBuf::new(),
            add_dirs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum CodexEvent {
    SessionStarted { session_id: String },
    Token { delta: String },
    Status { phase: String, message: Option<String> },
    Completed { finish_reason: Option<String>, session_id: Option<String> },
    Error { message: String },
}

pub struct CodexClient {
    config: CodexConfig,
}

impl CodexClient {
    pub fn new(config: CodexConfig) -> Self {
        Self { config }
    }

    pub fn send_message(
        &self,
        prompt: String,
        session_id: Option<String>,
        executor: BackgroundExecutor,
    ) -> (mpsc::Receiver<CodexEvent>, Task<Result<()>>) {
        let (mut tx, rx) = mpsc::channel::<CodexEvent>(100);
        let config = self.config.clone();

        let task = executor.spawn(async move {
            let result = run_codex_stream(config, prompt, session_id, &mut tx).await;
            if let Err(ref err) = result {
                let _ = tx.send(CodexEvent::Error {
                    message: err.to_string(),
                }).await;
            }
            result
        });

        (rx, task)
    }
}

async fn run_codex_stream(
    config: CodexConfig,
    prompt: String,
    session_id: Option<String>,
    tx: &mut mpsc::Sender<CodexEvent>,
) -> Result<()> {
    let mut command = Command::new(&config.cli_path);

    command.arg("exec");

    command
        .arg("--json")
        .arg("--dangerously-bypass-approvals-and-sandbox")
        .arg("--enable")
        .arg("shell_tool")
        .arg("--enable")
        .arg("apply_patch_freeform")
        .arg("--enable")
        .arg("web_search_request");

    for dir in &config.add_dirs {
        command.arg("--add-dir").arg(dir);
    }

    if let Some(ref sid) = session_id {
        command.arg("resume").arg(sid);
    }

    // Always provide the prompt via stdin (use `-` flag)
    command.arg("-");

    command
        .current_dir(&config.repo_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    log::info!("Spawning codex: {:?}", command);
    if let Ok(path) = std::env::var("PATH") {
        // Truncate for logging if very long
        let display_path = if path.len() > 500 {
            format!("{}...(truncated)", &path[..500])
        } else {
            path
        };
        log::info!("Codex PATH={}", display_path);
    }

    let mut child = command
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn codex: {e}"))?;

    log::info!("Codex process spawned successfully");

    // Spawn stdin writing in a separate task (like the working desktop code)
    if let Some(mut stdin) = child.stdin.take() {
        let prompt_clone = prompt.clone();
        smol::spawn(async move {
            let _ = stdin.write_all(prompt_clone.as_bytes()).await;
            let _ = stdin.write_all(b"\n").await;
            let _ = stdin.flush().await;
            // stdin is dropped here, closing it
        }).detach();
    }

    // Spawn stderr reading in a separate task
    if let Some(stderr) = child.stderr.take() {
        smol::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Some(line) = reader.next().await {
                if let Ok(line) = line {
                    if !line.trim().is_empty() {
                        log::warn!("[codex stderr] {}", line);
                    }
                }
            }
        }).detach();
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("codex stdout unavailable"))?;

    let mut reader = BufReader::new(stdout).lines();

    let mut assembled = String::new();
    let mut captured_session_id: Option<String> = None;

    log::info!("Starting to read codex stdout...");

    // Main loop only reads stdout
    while let Some(line) = reader.next().await {
        let line = line?;
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        log::info!("[codex stdout] {}", trimmed);

        let v: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Failed to parse codex JSON: {} - line: {}", e, trimmed);
                continue;
            }
        };

        let etype = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        log::info!("Codex event type: {}", etype);

        match etype {
            "thread.started" => {
                if let Some(tid) = v.get("thread_id").and_then(|t| t.as_str()) {
                    captured_session_id = Some(tid.to_string());
                    let _ = tx.send(CodexEvent::SessionStarted {
                        session_id: tid.to_string(),
                    }).await;
                }
                let _ = tx.send(CodexEvent::Status {
                    phase: "starting".to_string(),
                    message: Some("Starting session...".to_string()),
                }).await;
            }
            "turn.started" => {
                let _ = tx.send(CodexEvent::Status {
                    phase: "thinking".to_string(),
                    message: Some("Thinking...".to_string()),
                }).await;
            }
            "item.started" | "item.updated" | "item.completed" => {
                if let Some(item) = v.get("item") {
                    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");

                    if let Some((phase, message)) = describe_item_status(item_type, item) {
                        let _ = tx.send(CodexEvent::Status { phase, message }).await;
                    }

                    if (etype == "item.updated" || etype == "item.completed")
                        && item_type == "agent_message"
                    {
                        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                            if !text.is_empty() && !assembled.starts_with(text) {
                                if text.starts_with(&assembled) {
                                    let delta = text[assembled.len()..].to_string();
                                    if !delta.is_empty() {
                                        let _ = tx.send(CodexEvent::Token { delta }).await;
                                    }
                                    assembled = text.to_string();
                                } else {
                                    let _ = tx.send(CodexEvent::Token {
                                        delta: text.to_string(),
                                    }).await;
                                    assembled = text.to_string();
                                }
                            }
                        }
                    }
                }
            }
            "turn.completed" => {
                let _ = tx.send(CodexEvent::Status {
                    phase: "done".to_string(),
                    message: Some("Completed".to_string()),
                }).await;
                let _ = tx.send(CodexEvent::Completed {
                    finish_reason: Some("turn_completed".to_string()),
                    session_id: captured_session_id.clone(),
                }).await;
                break;
            }
            "turn.failed" | "error" => {
                let msg = v
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .or_else(|| v.get("message").and_then(|m| m.as_str()))
                    .unwrap_or("codex error");

                let _ = tx.send(CodexEvent::Error {
                    message: msg.to_string(),
                }).await;
                let _ = tx.send(CodexEvent::Completed {
                    finish_reason: Some(format!("error: {msg}")),
                    session_id: None,
                }).await;
                break;
            }
            _ => {}
        }
    }

    log::info!("Codex stdout loop ended, waiting for process...");
    let status = child.status().await;
    log::info!("Codex process exited with status: {:?}", status);
    Ok(())
}

fn describe_item_status(item_type: &str, item: &serde_json::Value) -> Option<(String, Option<String>)> {
    match item_type {
        "command_execution" => {
            let command = item
                .get("command")
                .and_then(|c| c.as_str())
                .map(|c| c.trim())
                .filter(|c| !c.is_empty());
            let msg = command
                .map(|c| format!("Running command `{c}`..."))
                .unwrap_or_else(|| "Running command...".to_string());
            Some(("running_command".to_string(), Some(msg)))
        }
        "file_change" | "workspace_patch" | "workspace_edit" | "file_edit" | "apply_patch" => {
            Some(("editing_files".to_string(), Some("Editing files...".to_string())))
        }
        "web_search" => {
            Some(("web_search".to_string(), Some("Searching the web...".to_string())))
        }
        "mcp_tool_call" | "tool_call" => {
            let tool_name = item
                .get("tool_name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    item.get("tool")
                        .and_then(|t| t.get("name"))
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_string())
                });
            let msg = tool_name
                .map(|name| format!("Calling tool: {name}..."))
                .unwrap_or_else(|| "Calling tool...".to_string());
            Some(("tool_call".to_string(), Some(msg)))
        }
        "plan_update" => {
            Some(("planning".to_string(), Some("Updating plan...".to_string())))
        }
        "reasoning" => {
            Some(("thinking".to_string(), Some("Reasoning...".to_string())))
        }
        _ => None,
    }
}
