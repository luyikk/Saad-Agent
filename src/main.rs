use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::CmdError::StringError;
use anyhow::Result;
use rig::agent::stream_to_stdout;
use rig::completion::ToolDefinition;
use rig::prelude::*;
use rig::providers::*;
use rig::streaming::StreamingChat;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::AsyncBufReadExt;
use tracing::level_filters::LevelFilter;
use tracing::*;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

// ── Permission levels ────────────────────────────────────────────
const PERM_PROMPT: u8 = 0; // 每次都询问
const PERM_SESSION_ALLOW_ALL: u8 = 1; // 当前流程全部允许
const PERM_PERMANENT_ALLOW_ALL: u8 = 2; // 永久允许（持久化）

static PERMISSION_LEVEL: AtomicU8 = AtomicU8::new(PERM_PROMPT);

fn perm_config_path() -> PathBuf {
    PathBuf::from(".saad-agent").join("permission.toml")
}

fn load_permanent_permission() {
    if let Ok(data) = std::fs::read_to_string(perm_config_path()) {
        if data.trim() == "allow_all" {
            PERMISSION_LEVEL.store(PERM_PERMANENT_ALLOW_ALL, Ordering::Relaxed);
        }
    }
}

fn save_permanent_permission() {
    let path = perm_config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, "allow_all");
}

/// Ask the user for permission. Returns Ok(()) if allowed, Err if denied.
async fn confirm_execution(cmdline: &str) -> Result<(), CmdError> {
    let level = PERMISSION_LEVEL.load(Ordering::Relaxed);
    match level {
        PERM_SESSION_ALLOW_ALL | PERM_PERMANENT_ALLOW_ALL => return Ok(()),
        _ => {}
    }

    println!();
    println!("╔══════════════════════════════════════════╗");
    println!("║  ⚠️  即将执行命令:                       ║");
    println!("║  📁 {cmdline}",);
    // Pad the command line display
    println!("╠══════════════════════════════════════════╣");
    println!("║  [y] 允许本次执行                       ║");
    println!("║  [a] 本次会话全部允许                   ║");
    println!("║  [p] 永久允许（不再询问）               ║");
    println!("║  [N] 拒绝                               ║");
    println!("╚══════════════════════════════════════════╝");
    print!("请选择 [y/a/p/N]: ");
    std::io::stdout().flush().ok();

    let mut confirmation = String::new();
    tokio::io::BufReader::new(tokio::io::stdin())
        .read_line(&mut confirmation)
        .await
        .map_err(|e| CmdError::StdError(e))?;

    match confirmation.trim().to_lowercase().as_str() {
        "y" | "yes" => Ok(()),
        "a" => {
            PERMISSION_LEVEL.store(PERM_SESSION_ALLOW_ALL, Ordering::Relaxed);
            println!("✅ 本次会话中所有命令将自动允许执行。");
            Ok(())
        }
        "p" => {
            PERMISSION_LEVEL.store(PERM_PERMANENT_ALLOW_ALL, Ordering::Relaxed);
            save_permanent_permission();
            println!(
                "✅ 已永久允许。如需恢复询问，请删除文件: {}",
                perm_config_path().display()
            );
            Ok(())
        }
        _ => Err(CmdError::StringError(format!(
            "User denied execution of: {cmdline}"
        ))),
    }
}

// ── Data types ───────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct OperationArgs {
    /// 完整的命令行语句，已包含所有参数（如 "Get-ChildItem -Path d:\"）
    command: String,
}

#[derive(Debug, thiserror::Error)]
#[error("cmd error")]
enum CmdError {
    #[error("error: {0}")]
    StdError(#[from] std::io::Error),
    #[error("error: {0}")]
    StringError(String),
}

#[derive(Deserialize, Serialize, Debug)]
struct RunCmd;

// ── Tool implementation ──────────────────────────────────────────

impl Tool for RunCmd {
    const NAME: &'static str = "RunCmd";

    type Error = CmdError;
    type Args = OperationArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "RunCmd".to_string(),
            description: r#"在本地操作系统中安全地执行完整的命令行语句。支持跨平台 (Windows/Linux/macOS)。
            【⚠️ 关键】command 必须是完整且可直接执行的单行命令字符串，已包含所有参数。
            - ✅ Windows 正确示例：`"Get-ChildItem -Path d:\\"`、`"dir d:\\"`
            - ✅ Linux/macOS 正确示例：`"ls -l /var/log"`、`"grep -r foo ./src"`
            - ❌ 错误示例：不要把命令和参数分成两个字段传入。
            【注意】Windows 使用 PowerShell 执行，Linux/macOS 使用 sh 执行。禁止执行破坏性或高危的恶意命令。"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "完整的命令行字符串，包含程序名和所有参数。Windows 下使用 PowerShell 语法，Linux/macOS 下使用 Bash/Shell 语法。"
                    }
                },
                "required": ["command"],
            }),
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let cmdline = args.command;

        // Permission gate
        confirm_execution(&cmdline).await?;

        trace!("Running command: '{cmdline}'");

        let output = if cfg!(target_os = "windows") {
            std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &format!(
                        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; {}",
                        cmdline
                    ),
                ])
                .output()
        } else {
            std::process::Command::new("sh")
                .args(["-c", &cmdline])
                .output()
        }
        .map_err(|e| CmdError::StdError(e))?;

        if output.status.success() {
            let out = String::from_utf8_lossy(&output.stdout);
            let err = String::from_utf8_lossy(&output.stderr);
            if out.is_empty() && !err.is_empty() {
                Err(StringError(err.to_string()))
            } else {
                if out.trim().is_empty() {
                    Ok("Command completed successfully with no output.".to_string())
                } else {
                    Ok(out.to_string())
                }
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let msg = if stderr.is_empty() { stdout } else { stderr };
            Err(StringError(format!(
                "Command '{}' failed with status {}: {}",
                cmdline,
                output.status,
                msg.trim(),
            )))
        }
    }
}

// ── Main ─────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let filter_layer = tracing_subscriber::filter::Targets::new()
        .with_default(Level::TRACE)
        .with_target("reqwest", LevelFilter::WARN)
        .with_target("hyper_util", LevelFilter::WARN)
        .with_target("h2", LevelFilter::WARN)
        .with_target("rig", LevelFilter::WARN);

    let fmt_layer = tracing_subscriber::fmt::layer()
        .pretty()
        .with_span_events(FmtSpan::CLOSE);

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .init();

    // Load persisted permission state
    load_permanent_permission();

    let client = deepseek::Client::new("sk-d8c73a5fda5b4df89b381c74689b3722")?; //ollama::Client::new(Nothing)?;

    let agent = client
        .agent("deepseek-v4-flash")
        .preamble(r#"你是一个专业的程序员!"#)
        .name("Saad")
        .default_max_turns(100)
        .temperature(0.5)
        .max_tokens(4096)
        .tool(RunCmd)
        .build();

    let mut history = vec![];

    loop {
        print!(">");
        std::io::stdout().flush()?;
        let mut prompt = String::new();
        tokio::io::BufReader::new(tokio::io::stdin())
            .read_line(&mut prompt)
            .await?;

        let mut stream = agent.stream_chat(prompt, &history).await;
        let res = stream_to_stdout(&mut stream).await?;

        history.extend_from_slice(res.history().unwrap_or_default());
        println!();
        println!("Token usage response: {usage:?}", usage = res.usage());
    }
}
