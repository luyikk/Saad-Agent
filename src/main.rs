//! Saad Agent — AI 编程助手
//!
//! 基于 DeepSeek 模型的智能命令行助手，可执行系统命令、
//! 读写文件来帮助用户完成编程任务。

use std::io::Write;

use anyhow::Result;
use console::style;
use futures_util::stream::StreamExt;
use rig::agent::{CompletionCall, FinalResponse, MultiTurnStreamItem, Text};
use rig::message::{Message, ToolCall, ToolResult, ToolResultContent};
use rig::prelude::*;
use rig::providers::deepseek;
use rig::providers::deepseek::Client as DeepSeekClient;
use rig::streaming::{StreamedAssistantContent, StreamedUserContent, StreamingChat};
use tokio::io::AsyncBufReadExt;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

mod command;
mod config;
mod error;
mod history;
mod permission;
mod tool;
mod ui;

#[tokio::main]
async fn main() -> Result<()> {
    // ---- 初始化日志系统 ----
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

    // ---- 加载 .env 文件 ----
    let _ = dotenvy::dotenv();

    // ---- 加载持久化的权限状态 ----
    permission::load_permanent_permission();

    // ---- 获取 API Key ----
    let api_key = config::get_api_key().map_err(|e| anyhow::anyhow!(e))?;
    let client: DeepSeekClient = deepseek::Client::new(&api_key)?;

    // ---- 构建 AI Agent ----
    let model_name = config::get_model_name();
    tracing::info!("使用模型: {}", model_name);

    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "未知".to_string());

    let preamble = format!(
        r#"你是一个专业的程序员助手，可以执行命令和读写文件来帮助用户完成任务。

【当前工作目录】
{}

【注意事项】
- 所有相对路径都基于上述工作目录
- 在执行命令或读写文件时，优先使用绝对路径
- 如果不确定某个文件的位置，先用 Get-ChildItem / ls 探索目录结构"#,
        cwd
    );

    let agent = client
        .agent(&model_name)
        .preamble(&preamble)
        .name("Saad")
        .default_max_turns(config::DEFAULT_MAX_TURNS)
        .temperature(config::DEFAULT_TEMPERATURE)
        .max_tokens(config::get_max_tokens() as u64)
        .tool(tool::cmd::RunCmd)
        .tool(tool::fs::ReadFile)
        .tool(tool::fs::WriteFile)
        .build();

    // ---- 加载对话历史 ----
    let mut history: Vec<Message> = history::load_history().unwrap_or_else(|e| {
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
        // 美化提示符
        print!("{} ", style("❯").cyan().bold());
        std::io::stdout().flush()?;

        let mut prompt = String::new();

        // tokio::select! 同时等待输入和 Ctrl+C
        let read_result = tokio::select! {
            result = reader.read_line(&mut prompt) => Some(result),
            _ = tokio::signal::ctrl_c() => None,
        };

        match read_result {
            None => {
                // Ctrl+C → 优雅退出
                println!();
                if !history.is_empty() {
                    if let Err(e) = history::save_history(&history) {
                        tracing::warn!("保存对话历史失败: {}", e);
                    }
                }
                ui::print_goodbye(!history.is_empty());
                break;
            }
            Some(Err(e)) => {
                ui::print_error(&format!("读取输入失败: {}", e));
                break;
            }
            Some(Ok(0)) => {
                // EOF
                println!();
                break;
            }
            Some(Ok(_)) => {}
        }

        let prompt = prompt.trim().to_string();

        // 跳过空输入
        if prompt.is_empty() {
            continue;
        }

        // ---- 处理内置命令 ----
        if prompt.starts_with('/') {
            if let Some(true) = command::handle_command(&prompt, &mut history, max_history).await {
                break; // 退出
            }
            continue;
        }

        // ---- 发送消息并流式输出 ----
        tracing::debug!("发送消息 (历史长度: {})", history.len());

        // 显示 spinner 等待首个 token
        let spinner = ui::new_spinner("AI 正在思考...");
        let mut stream = agent.stream_chat(prompt, &history).await;
        spinner.finish_and_clear();

        // ── 流式渲染器 ──
        let mut display = ui::StreamDisplay::new();
        let mut final_res = FinalResponse::empty();

        while let Some(content) = stream.next().await {
            match content {
                // ── 回答文本增量 ──
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    display.on_answer(&text)?;
                }

                // ── 推理链增量（流式） ──
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. },
                )) => {
                    display.on_reasoning_delta(&reasoning)?;
                }

                // ── 推理链完整块 ──
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::Reasoning(reasoning),
                )) => {
                    display.on_reasoning(&reasoning.display_text())?;
                }

                // ── 工具调用 ──
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::ToolCall {
                        tool_call: ToolCall { function, .. },
                        ..
                    },
                )) => {
                    let args_preview =
                        serde_json::to_string(&function.arguments).unwrap_or_default();
                    display.on_tool_call(&function.name, &args_preview)?;
                }

                // ── 工具调用参数增量 ──
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::ToolCallDelta { content, .. },
                )) => {
                    use rig::streaming::ToolCallDeltaContent;
                    match content {
                        ToolCallDeltaContent::Name(name) => {
                            display.on_tool_call_delta(&format!("[调用: {}]", name))?;
                        }
                        ToolCallDeltaContent::Delta(delta) => {
                            display.on_tool_call_delta(&delta)?;
                        }
                    }
                }

                // ── 工具返回结果 ──
                Ok(MultiTurnStreamItem::StreamUserItem(StreamedUserContent::ToolResult {
                    tool_result: ToolResult { content, .. },
                    ..
                })) => {
                    let summary = content
                        .iter()
                        .filter_map(|c| match c {
                            ToolResultContent::Text(t) => Some(t.text.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let success = !summary.to_lowercase().contains("error")
                        && !summary.to_lowercase().contains("failed");
                    let preview = if summary.len() > 200 {
                        format!("{}...", &summary[..200])
                    } else {
                        summary
                    };
                    display.on_tool_result(success, &preview)?;
                }

                // ── Provider Completion 调用详情 ──
                Ok(MultiTurnStreamItem::CompletionCall(CompletionCall {
                    call_index,
                    usage,
                    ..
                })) => {
                    display.on_completion_call(call_index as u32, usage);
                }

                // ── Agent 最终响应 ──
                Ok(MultiTurnStreamItem::FinalResponse(res)) => {
                    final_res = res;
                }

                // ── 流错误 ──
                Err(err) => {
                    display.on_error(&format!("AI 响应流错误: {err}"));
                }

                // ── Provider 最终响应 / 其他（忽略） ──
                _ => {}
            }
        }

        // 打印 Token 统计
        display.finalize(&final_res.usage());

        // 更新对话历史
        if let Some(new_history) = final_res.history() {
            history.extend_from_slice(new_history);
        }

        // 限制历史长度
        history::trim_history(&mut history, max_history);
    }

    std::process::exit(0);
}
