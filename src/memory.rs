//! 对话历史持久化模块
//!
//! 负责消息的序列化/反序列化、保存到 JSON 文件、加载以及长度裁剪。
//! `rig::message::Message` 原生支持 `Serialize + Deserialize`，无需中间类型。

use anyhow::Result;
use rig::completion::AssistantContent;
use rig::message::{Message, UserContent};

use crate::config;



// ============================================================
// 消息文本提取
// ============================================================

/// 从 Message 中提取纯文本内容（含 `ToolCall` 标记）
pub fn message_text(msg: &Message) -> String {
    match msg {
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
                AssistantContent::Image(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// 从 Message 中提取文本用于预览（限制最大字符数）
pub fn message_preview(msg: &Message, max_chars: usize) -> String {
    let text = message_text(msg);
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

/// 获取消息的角色名称
pub const fn message_role_name(msg: &Message) -> &'static str {
    match msg {
        Message::System { .. } => "system",
        Message::User { .. } => "user",
        Message::Assistant { .. } => "assistant",
    }
}

// ============================================================
// 持久化 I/O
// ============================================================

/// 保存对话历史到 JSON 文件
pub fn save_history(history: &[Message]) -> Result<()> {
    let path = config::history_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(history)?;
    std::fs::write(&path, json)?;
    tracing::debug!("对话历史已保存到: {} ({} 条消息)", path.display(), history.len());
    Ok(())
}

/// 从 JSON 文件加载对话历史
pub fn load_history() -> Result<Vec<Message>> {
    let path = config::history_path();
    if !path.exists() {
        return Ok(vec![]);
    }
    let json = std::fs::read_to_string(&path)?;
    let messages: Vec<Message> = serde_json::from_str(&json)?;
    tracing::debug!("从 {} 加载了 {} 条历史消息", path.display(), messages.len());
    Ok(messages)
}

// ============================================================
// 工具函数
// ============================================================

/// 限制对话历史长度，保留最近的 N 条消息
pub fn trim_history(history: &mut Vec<Message>, max: usize) {
    if history.len() > max {
        let remove = history.len() - max;
        history.drain(0..remove);
        tracing::debug!("对话历史已截断，移除了 {} 条旧消息", remove);
    }
}
