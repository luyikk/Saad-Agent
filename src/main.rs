use std::io::Write;

use anyhow::Result;
use rig::agent::stream_to_stdout;
use rig::completion::{AssistantContent, Message, Usage};
use rig::message::{Text, UserContent};
use rig::prelude::*;
use rig::providers::deepseek;
use rig::providers::deepseek::Client as DeepSeekClient;
use rig::streaming::StreamingChat;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

mod config;
mod permission;
mod tool;

// ============================================================
// 对话历史持久化类型
// ============================================================

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SavedMessage {
    role: String,
    content: String,
}

impl SavedMessage {
    fn from_rig(msg: &Message) -> Self {
        let (role, content) = match msg {
            Message::System { content } => ("system".to_string(), content.clone()),
            Message::User { content } => {
                let text = content
                    .iter()
                    .filter_map(|c| match c {
                        UserContent::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                ("user".to_string(), text)
            }
            Message::Assistant { content, .. } => {
                let text = content
                    .iter()
                    .filter_map(|c| match c {
                        AssistantContent::Text(t) => Some(t.text.clone()),
                        AssistantContent::Reasoning(r) => Some(r.display_text()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                ("assistant".to_string(), text)
            }
        };
        SavedMessage { role, content }
    }

    fn to_rig(&self) -> Message {
        match self.role.as_str() {
            "system" => Message::System {
                content: self.content.clone(),
            },
            "user" => Message::User {
                content: rig::one_or_many::OneOrMany::one(UserContent::Text(Text::new(
                    self.content.clone(),
                ))),
            },
            _ => Message::Assistant {
                id: None,
                content: rig::one_or_many::OneOrMany::one(AssistantContent::Text(Text::new(
                    self.content.clone(),
                ))),
            },
        }
    }
}

/// 保存对话历史到 JSON 文件
fn save_history(history: &[Message]) -> Result<()> {
    let path = config::history_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let saved: Vec<SavedMessage> = history.iter().map(SavedMessage::from_rig).collect();
    let json = serde_json::to_string_pretty(&saved)?;
    std::fs::write(&path, json)?;
    tracing::debug!("对话历史已保存到: {}", path.display());
    Ok(())
}

/// 从 JSON 文件加载对话历史
fn load_history() -> Result<Vec<Message>> {
    let path = config::history_path();
    if !path.exists() {
        return Ok(vec![]);
    }
    let json = std::fs::read_to_string(&path)?;
    let saved: Vec<SavedMessage> = serde_json::from_str(&json)?;
    let messages: Vec<Message> = saved.iter().map(SavedMessage::to_rig).collect();
    tracing::debug!("从 {} 加载了 {} 条历史消息", path.display(), messages.len());
    Ok(messages)
}

/// 限制对话历史长度，保留最近的 N 条消息
fn trim_history(history: &mut Vec<Message>, max: usize) {
    if history.len() > max {
        let remove = history.len() - max;
        history.drain(0..remove);
        tracing::debug!("对话历史已截断，移除了 {} 条旧消息", remove);
    }
}

/// 从 Message 中提取文本用于预览
fn message_preview(msg: &Message, max_chars: usize) -> String {
    let text = match msg {
        Message::System { content } => content.clone(),
        Message::User { content } => content
            .iter()
            .filter_map(|c| match c {
                UserContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Message::Assistant { content, .. } => content
            .iter()
            .filter_map(|c| match c {
                AssistantContent::Text(t) => Some(t.text.clone()),
                AssistantContent::Reasoning(r) => Some(r.display_text()),
                AssistantContent::ToolCall(tc) => Some(format!("[ToolCall: {}]", tc.function.name)),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    };

    let chars: Vec<char> = text.chars().collect();
    if chars.len() > max_chars {
        format!(
            "{}...",
            chars.into_iter().take(max_chars).collect::<String>()
        )
    } else {
        text
    }
}

fn message_role_name(msg: &Message) -> &'static str {
    match msg {
        Message::System { .. } => "system",
        Message::User { .. } => "user",
        Message::Assistant { .. } => "assistant",
    }
}

// ============================================================
// 打印帮助信息
// ============================================================

fn print_help() {
    println!();
    println!("╔══════════════════════════════════════════════╗");
    println!("║   Saad Agent 帮助                          ║");
    println!("╠══════════════════════════════════════════════╣");
    println!("║  /help    - 显示此帮助信息                 ║");
    println!("║  /clear   - 清空对话历史                   ║");
    println!("║  /save    - 保存对话历史到磁盘             ║");
    println!("║  /load    - 从磁盘加载对话历史             ║");
    println!("║  /history - 显示当前对话历史统计           ║");
    println!("║  /exit    - 退出程序                       ║");
    println!("║  Ctrl+C   - 优雅退出（自动保存历史）       ║");
    println!("╚══════════════════════════════════════════════╝");
    println!();
}

// ============================================================
// Main
// ============================================================

#[tokio::main]
async fn main() -> Result<()> {
    // ---- 初始化日志系统 ----
    let filter_layer = tracing_subscriber::filter::Targets::new()
        .with_default(tracing::Level::INFO)
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

    // ---- 加载 .env 文件（可选） ----
    let _ = dotenvy::dotenv();

    // ---- 加载持久化的权限状态 ----
    permission::load_permanent_permission();

    // ---- 获取 API Key ----
    let api_key = config::get_api_key().map_err(|e| anyhow::anyhow!(e))?;
    let client: DeepSeekClient = deepseek::Client::new(&api_key)?;

    // ---- 构建 AI Agent ----
    let model_name = config::get_model_name();
    tracing::info!("使用模型: {}", model_name);

    let agent = client
        .agent(&model_name)
        .preamble("你是一个专业的程序员助手，可以执行命令和读写文件来帮助用户完成任务。")
        .name("Saad")
        .default_max_turns(config::DEFAULT_MAX_TURNS)
        .temperature(config::DEFAULT_TEMPERATURE)
        .max_tokens(config::DEFAULT_MAX_TOKENS as u64)
        .tool(tool::cmd::RunCmd)
        .tool(tool::fs::ReadFile)
        .tool(tool::fs::WriteFile)
        .build();

    // ---- 尝试加载持久化的对话历史 ----
    let mut history: Vec<Message> = load_history().unwrap_or_else(|e| {
        tracing::warn!("加载对话历史失败: {}，将使用全新对话", e);
        vec![]
    });

    let max_history = config::get_max_history_messages();

    println!();
    println!("╔══════════════════════════════════════════╗");
    println!("║   Saad Agent - AI 编程助手              ║");
    println!("║   输入你的需求，我会帮你完成！           ║");
    println!("║   输入 /help 查看命令列表               ║");
    println!("╚══════════════════════════════════════════╝");

    if !history.is_empty() {
        println!("📂 已加载 {} 条历史消息", history.len());
    }

    // ---- 主交互循环 ----
    let stdin = tokio::io::stdin();
    let mut reader = tokio::io::BufReader::new(stdin);

    loop {
        print!("\n> ");
        std::io::stdout().flush()?;

        let mut prompt = String::new();

        // 使用 tokio::select! 同时等待输入和 Ctrl+C
        let read_result = tokio::select! {
            result = reader.read_line(&mut prompt) => {
                Some(result)
            }
            _ = tokio::signal::ctrl_c() => {
                None // Ctrl+C 被按下
            }
        };

        match read_result {
            None => {
                // Ctrl+C 优雅退出
                println!("\n\n👋 正在退出...");
                if !history.is_empty() {
                    if let Err(e) = save_history(&history) {
                        tracing::warn!("保存对话历史失败: {}", e);
                    } else {
                        println!("💾 对话历史已自动保存");
                    }
                }
                break;
            }
            Some(Err(e)) => {
                tracing::error!("读取输入失败: {}", e);
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
            match prompt.to_lowercase().as_str() {
                "/exit" | "/quit" => {
                    println!("👋 再见！");
                    if !history.is_empty() {
                        let _ = save_history(&history);
                    }
                    break;
                }
                "/help" => {
                    print_help();
                    continue;
                }
                "/clear" => {
                    history.clear();
                    let _ = std::fs::remove_file(config::history_path());
                    println!("🧹 对话历史已清空");
                    continue;
                }
                "/save" => {
                    if history.is_empty() {
                        println!("⚠️  对话历史为空，无需保存");
                    } else {
                        match save_history(&history) {
                            Ok(()) => {
                                println!("💾 对话历史已保存 ({} 条消息)", history.len())
                            }
                            Err(e) => println!("❌ 保存失败: {}", e),
                        }
                    }
                    continue;
                }
                "/load" => {
                    match load_history() {
                        Ok(loaded) => {
                            if loaded.is_empty() {
                                println!("⚠️  没有找到保存的对话历史");
                            } else {
                                history = loaded;
                                println!("📂 已加载 {} 条历史消息", history.len());
                            }
                        }
                        Err(e) => println!("❌ 加载失败: {}", e),
                    }
                    continue;
                }
                "/history" => {
                    if history.is_empty() {
                        println!("📝 当前对话历史为空");
                    } else {
                        println!(
                            "📝 当前对话历史: {} 条消息 (限制: {} 条)",
                            history.len(),
                            max_history
                        );
                        let start = if history.len() > 5 {
                            history.len() - 5
                        } else {
                            0
                        };
                        for (i, msg) in history.iter().enumerate().skip(start) {
                            let role = message_role_name(msg);
                            let preview = message_preview(msg, 60);
                            println!("  [{i}] {role}: {preview}");
                        }
                    }
                    continue;
                }
                _ => {
                    println!("❓ 未知命令: {prompt}。输入 /help 查看可用命令");
                    continue;
                }
            }
        }

        // ---- 发送消息并流式输出回复 ----
        tracing::debug!("发送消息 (历史长度: {})", history.len());
        let mut stream = agent.stream_chat(prompt, &history).await;
        let res = stream_to_stdout(&mut stream).await?;

        // 更新对话历史
        if let Some(new_history) = res.history() {
            history.extend_from_slice(new_history);
        }

        // 限制历史长度
        trim_history(&mut history, max_history);

        // 打印 Token 使用统计
        let usage: Usage = res.usage();
        tracing::debug!("Token 使用情况: {:?}", usage);
    }

    Ok(())
}
