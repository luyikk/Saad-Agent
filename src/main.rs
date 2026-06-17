//! Saad Agent — AI 编程助手
//!
//! 基于 `DeepSeek` 模型的智能命令行助手，可执行系统命令、
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
mod memory;
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
    let mut agent = build_agent(&client);

    // ---- 加载对话历史 ----
    let max_history = config::get_max_history_messages();
    let (loaded_messages, loaded_summary) = memory::ConversationMemory::load_from_disk()
        .unwrap_or_else(|e| {
            tracing::warn!("加载对话历史失败: {}，将使用全新对话", e);
            (vec![], None)
        });
    let mut memory =
        memory::ConversationMemory::from_parts(loaded_messages, loaded_summary, max_history);

    // ---- 欢迎界面 ----
    ui::print_welcome(memory.len());

    // ---- 主交互循环 ----
    let stdin = tokio::io::stdin();
    let mut reader = tokio::io::BufReader::new(stdin);

    loop {
        // 提示符
        print!("{} ", style("❯").cyan().bold());
        std::io::stdout().flush()?;

        let Some(prompt) = read_input(&mut reader).await else {
            save_and_exit(&memory)
        };

        if prompt.is_empty() {
            continue;
        }

        // 内置斜杠命令
        if prompt.starts_with('/') {
            match command::handle_command(&prompt, &mut memory, max_history)? {
                command::CommandResult::Exit => break,
                command::CommandResult::RebuildAgent => {
                    agent = build_agent(&client);
                }
                command::CommandResult::Continue => {}
            }
            continue;
        }

        // 构建上下文消息（摘要 + 当前消息）
        let context = memory.build_context();

        // ---- 发送消息并流式输出 ----
        tracing::debug!("发送消息 (历史长度: {})", context.len());
        let mut display = ui::StreamDisplay::new(100);

        let final_res = stream_handler::process_stream(
            &prompt,
            agent.stream_chat(&prompt, &context).await,
            &mut display,
        )
        .await?;

        // 更新对话历史
        if let Some(new_history) = final_res.history() {
            memory.extend(new_history);
        }

        // 智能压缩（超过 max_history 时用 AI 摘要）
        let compact_model = client.completion_model(config::get_model_name());
        memory.compact(&compact_model).await?;
    }

    std::process::exit(0);
}

// ── 辅助函数 ──

/// 初始化 tracing 日志系统
fn init_tracing() {
    let filter_layer = tracing_subscriber::filter::Targets::new()
        .with_default(tracing::Level::INFO)
        .with_target("reqwest", LevelFilter::WARN)
        .with_target("hyper_util", LevelFilter::WARN)
        .with_target("h2", LevelFilter::WARN)
        .with_target("rig", LevelFilter::WARN);
    //.with_target("saad_agent", LevelFilter::TRACE);

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

    let cwd =
        std::env::current_dir().map_or_else(|_| "未知".to_string(), |p| p.display().to_string());

    let mut notes = vec![
        "- 所有相对路径都基于上述工作目录",
        "- 在执行命令或读写文件时，优先使用绝对路径",
        "- 如果不确定某个文件的位置，先用 Get-ChildItem / ls 探索目录结构",
        "- 如果创建了临时文件用于某个命令，执行完后请及时删除以免混乱",
    ];

    // Windows + 非 PowerShell 7 → 禁止 && 语法
    if cfg!(target_os = "windows") && !ps_supports_and_and() {
        notes.push(
            "- 当前环境为 Windows PowerShell 5.1，绝对禁止使用 '&&' 或 '||' 连接命令！请用 ';' 分隔或分次执行。"
        );
    }

    let effort = config::get_effort_level();

    let preamble = format!(
        r#"你是一个专业的程序员助手，可以执行命令和读写文件来帮助用户完成任务。

        【当前工作目录】
        {}

        【可用工具】
        - ReadFile：读取指定路径的文件内容，返回带行号的内容
        - WriteFile：覆盖写入指定路径的文件（⚠️ 会覆盖已有内容）
        - EditFile：精确编辑文件，查找并替换指定文本片段（old_string 必须唯一匹配）
        - GetFileLines：获取文件总行数，用于评估文件规模
        - ExecuteCommand：执行完整的命令行语句，支持 Windows/Linux/macOS

        【变更策略 ⚠️ 必须遵守】
        每次修改代码前，必须先在回答中列出变更计划：
        1. 说明要修改哪些文件
        2. 每个文件的修改目的和内容概述
        3. 预估修改步骤数

        修改过程中逐步骤报告进度（如 "✅ 步骤 1/3: 已完成 xxx"）。
        全部修改完成后，总结实际变更内容。

        【注意事项】
        {}

        【回答风格】
        {}"#,
        cwd,
        notes.join("\n"),
        effort.preamble_instruction(),
    );

    tracing::info!(
        "Effort level: {:?}, max_tokens: {}",
        effort,
        effort.max_tokens()
    );

    client
        .agent(&model_name)
        .preamble(&preamble)
        .name("Saad")
        .default_max_turns(config::get_max_turns())
        .temperature(config::DEFAULT_TEMPERATURE)
        .max_tokens(effort.max_tokens() as u64)
        .tool(tool::cmd::ExecuteCommand)
        .tool(tool::fs::ReadFile)
        .tool(tool::fs::WriteFile)
        .tool(tool::fs::EditFile)
        .tool(tool::fs::GetFileLines)
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
        .is_some_and(|s| s.trim() == "True")
}

/// 读取一行用户输入，返回 `None` 表示 EOF/Ctrl+C
async fn read_input(reader: &mut tokio::io::BufReader<tokio::io::Stdin>) -> Option<String> {
    let mut prompt = String::new();

    let read_result = tokio::select! {
        result = tokio::io::AsyncBufReadExt::read_line(reader, &mut prompt) => Some(result),
        _ = tokio::signal::ctrl_c() => None,
    };

    match read_result {
        Some(Err(e)) => {
            ui::print_error(&format!("读取输入失败: {e}"));
            None
        }
        None | Some(Ok(0)) => {
            println!();
            None
        }
        Some(Ok(_)) => Some(prompt.trim().to_string()),
    }
}

/// 保存历史并优雅退出
fn save_and_exit(mem: &memory::ConversationMemory) -> ! {
    if !mem.is_empty() {
        if let Err(e) = mem.save_to_disk() {
            tracing::warn!("保存对话历史失败: {}", e);
        }
    }
    ui::print_goodbye(!mem.is_empty());
    std::process::exit(0);
}
