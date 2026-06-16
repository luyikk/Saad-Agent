use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

use anyhow::Result;
use rig::agent::stream_to_stdout;
use rig::client::Nothing;
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
        .map_err(|e| CmdError::ToolCallError(Box::new(e)))?;

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
        _ => Err(CmdError::ToolCallError(Box::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("User denied execution of: {cmdline}"),
        )))),
    }
}

// ── Data types ───────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct OperationArgs {
    cmd: String,
    args: serde_json::value::Value,
}

#[derive(Debug, thiserror::Error)]
#[error("cmd error")]
enum CmdError {
    #[error("Tool call error: {0}")]
    ToolCallError(#[from] Box<dyn std::error::Error + Send + Sync + 'static>),
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
            description: r#"在本地操作系统中安全地执行系统命令或脚本。支持跨平台 (Windows/Linux/macOS)。
            【⚠️ 关键规则】你必须将主命令与参数严格分离！
            - ✅ Windows 正确示例：执行目录列表 -> cmd: "Get-ChildItem", args: ["-Path", "d:\"]
            - ✅ Linux 正确示例：执行长列表 -> cmd: "ls", args: ["-l", "/var/log"]
            - ❌ 错误示例：cmd: "dir /b d:\\" , args: [] （绝对不要把参数拼接到 cmd 字段中）
            【注意】如果命令不需要任何参数，args 必须传入空数组 []。禁止执行破坏性或高危的恶意命令。"#.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "cmd": {
                        "type": "string",
                        "description": "要执行的主程序或命令名称。在 Windows 环境下，默认运行 PowerShell 环境，请使用 PowerShell 语法和命令（如 'Get-ChildItem', 'Write-Host'）；在 Linux/macOS 下使用 Bash/Zsh 命令（如 'ls', 'echo'）。不要包含任何参数"
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "命令行参数数组，每个参数必须是独立的字符串元素。无参数时传空数组 []。在 Windows 下传递文件路径时，直接使用单反斜杠 '\\' 即可（例如 'd:\\test'），不要使用双反斜杠 '\\\\'"
                    }
                },
                "required": ["cmd", "args"],
            }),
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let cmd = args.cmd;
        let cmd_args: Vec<String> = serde_json::from_value(args.args).unwrap_or_default();

        let cmdline = if cmd_args.is_empty() {
            cmd.clone()
        } else {
            format!("{} {}", cmd, cmd_args.join(" "))
        };

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
            std::process::Command::new(&cmd).args(&cmd_args).output()
        }
        .map_err(|e| CmdError::ToolCallError(Box::new(e)))?;

        if output.status.success() {
            let out = String::from_utf8_lossy(&output.stdout);
            let err = String::from_utf8_lossy(&output.stderr);
            Ok(if out.is_empty() && !err.is_empty() {
                err.to_string()
            } else {
                if out.trim().is_empty() {
                    "Command completed successfully with no output.".to_string()
                } else {
                    out.to_string()
                }
            })
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let msg = if stderr.is_empty() { stdout } else { stderr };
            Err(CmdError::ToolCallError(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "Command '{}' failed with status {}: {}",
                    cmdline,
                    output.status,
                    msg.trim(),
                ),
            ))))
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
