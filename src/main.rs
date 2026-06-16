//! Saad Agent — AI 编程助手
//!
//! 基于 DeepSeek 模型的智能命令行助手，可执行系统命令、
//! 读写文件来帮助用户完成编程任务。

use std::io::Write;

use anyhow::Result;
use console::style;
use rig::prelude::*;
use rig::providers::deepseek;
use rig::streaming::StreamingChat;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

mod command;
mod config;
mod error;
mod history;
mod permission;
mod stream_handler;
mod tool;
mod ui;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let _ = dotenvy::dotenv();
    permission::load_permanent_permission();

    // ---- 构建 AI Agent ----
    let api_key = config::get_api_key().map_err(|e| anyhow::anyhow!(e))?;
    let client = deepseek::Client::new(&api_key)?;
    let agent = build_agent(&client);

    // ---- 加载对话历史 ----
    let mut history = history::load_history().unwrap_or_else(|e| {
        tracing::warn!("加载对话历史失败: {}，将使用全新对话", e);
        vec![]
    });
    let max_history = config::get_max_history_messages();

    // ---- 欢迎界面 ----
    ui::print_welcome(history.len());

    // ---- 主交互循环 ----
    let stdin = tokio::io::stdin();
    let mut reader = tokio::io::BufReader::new(stdin);

    loop {
        // 提示符
        print!("{} ", style("❯").cyan().bold());
        std::io::stdout().flush()?;

        let prompt = match read_input(&mut reader).await {
            Some(p) => p,
            None => save_and_exit(&history),
        };

        if prompt.is_empty() {
            continue;
        }

        // 内置斜杠命令
        if prompt.starts_with('/') {
            if let Some(true) = command::handle_command(&prompt, &mut history, max_history).await {
                break;
            }
            continue;
        }

        // ---- 发送消息并流式输出 ----
        tracing::debug!("发送消息 (历史长度: {})", history.len());
        let mut display = ui::StreamDisplay::new();

        let final_res = stream_handler::process_stream(
            &prompt,
            agent.stream_chat(&prompt, &history).await,
            &mut display,
        )
        .await;

        // 更新对话历史
        if let Some(new_history) = final_res.history() {
            history.extend_from_slice(new_history);
        }
        history::trim_history(&mut history, max_history);
    }

    std::process::exit(0);
}

// ── 辅助函数 ──

/// 初始化 tracing 日志系统
fn init_tracing() {
    let filter_layer = tracing_subscriber::filter::Targets::new()
        .with_default(tracing::Level::TRACE)
        .with_target("reqwest", LevelFilter::WARN)
        .with_target("hyper_util", LevelFilter::WARN)
        .with_target("h2", LevelFilter::WARN)
        .with_target("rig", LevelFilter::WARN)
        .with_target("saad_agent", LevelFilter::TRACE);

    let fmt_layer = tracing_subscriber::fmt::layer()
        .pretty()
        .with_span_events(FmtSpan::CLOSE);

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .init();
}

/// 构建 AI Agent 实例
fn build_agent(client: &deepseek::Client) -> rig::agent::Agent<deepseek::CompletionModel> {
    let model_name = config::get_model_name();
    tracing::info!("使用模型: {}", model_name);

    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "未知".to_string());

    let mut notes = vec![
        "- 所有相对路径都基于上述工作目录".to_string(),
        "- 在执行命令或读写文件时，优先使用绝对路径".to_string(),
        "- 如果不确定某个文件的位置，先用 Get-ChildItem / ls 探索目录结构".to_string(),
    ];

    // Windows + 非 PowerShell 7 → 禁止 && 语法
    if cfg!(target_os = "windows") && !ps_supports_and_and() {
        notes.push(
            "- 当前环境为 Windows PowerShell 5.1，绝对禁止使用 '&&' 或 '||' 连接命令！请用 ';' 分隔或分次执行。"
                .to_string(),
        );
    }

    let preamble = format!(
        r#"你是一个专业的程序员助手，可以执行命令和读写文件来帮助用户完成任务。

【当前工作目录】
{}

【注意事项】
{}"#,
        cwd,
        notes.join("\n"),
    );

    client
        .agent(&model_name)
        .preamble(&preamble)
        .name("Saad")
        .default_max_turns(config::DEFAULT_MAX_TURNS)
        .temperature(config::DEFAULT_TEMPERATURE)
        .max_tokens(config::get_max_tokens() as u64)
        .tool(tool::cmd::RunCmd)
        .tool(tool::fs::ReadFile)
        .tool(tool::fs::WriteFile)
        .build()
}

/// 检测当前 PowerShell 是否支持 `&&` 连接语法（PS 7+ 才支持）
fn ps_supports_and_and() -> bool {
    std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$PSVersionTable.PSVersion.Major -ge 7",
        ])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim() == "True")
        .unwrap_or(false)
}

/// 读取一行用户输入，返回 `None` 表示 EOF/Ctrl+C
async fn read_input(reader: &mut tokio::io::BufReader<tokio::io::Stdin>) -> Option<String> {
    let mut prompt = String::new();

    let read_result = tokio::select! {
        result = tokio::io::AsyncBufReadExt::read_line(reader, &mut prompt) => Some(result),
        _ = tokio::signal::ctrl_c() => None,
    };

    match read_result {
        None => {
            println!();
            None
        }
        Some(Err(e)) => {
            ui::print_error(&format!("读取输入失败: {}", e));
            None
        }
        Some(Ok(0)) => {
            println!();
            None
        }
        Some(Ok(_)) => Some(prompt.trim().to_string()),
    }
}

/// 保存历史并优雅退出
fn save_and_exit(history: &[rig::message::Message]) -> ! {
    if !history.is_empty() {
        if let Err(e) = history::save_history(history) {
            tracing::warn!("保存对话历史失败: {}", e);
        }
    }
    ui::print_goodbye(!history.is_empty());
    std::process::exit(0);
}
