//! 对话记忆体模块
//!
//! 基于 AI 摘要的智能记忆压缩：当消息数超过 `max_messages` 时，
//! 调用模型对旧消息做摘要，避免直接截断丢失上下文。

use anyhow::Result;
use rig::completion::{AssistantContent, CompletionModel};
use rig::message::{Message, UserContent};

use crate::config;

// ============================================================
// ConversationMemory
// ============================================================

/// 智能对话记忆体
///
/// 消息超过上限时自动调用 AI 模型压缩旧消息为摘要，
/// 摘要以 System Message 形式注入后续对话。
pub struct ConversationMemory {
    messages: Vec<Message>,
    max_messages: usize,
    summary: Option<String>,
}

#[allow(dead_code)]
impl ConversationMemory {
    /// 创建新的记忆体
    pub fn new(max_messages: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_messages,
            summary: None,
        }
    }

    /// 从已有消息列表创建（用于加载历史）
    pub fn from_parts(messages: Vec<Message>, summary: Option<String>, max_messages: usize) -> Self {
        Self {
            messages,
            max_messages,
            summary,
        }
    }

    // ── 基本访问 ──

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn messages_mut(&mut self) -> &mut Vec<Message> {
        &mut self.messages
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.summary = None;
    }

    pub fn extend(&mut self, new_messages: &[Message]) {
        self.messages.extend_from_slice(new_messages);
    }

    pub fn set_max_messages(&mut self, max: usize) {
        self.max_messages = max;
    }

    // ── 摘要 ──

    /// 构建发送给 AI 的完整上下文消息列表（摘要 + 当前消息）
    pub fn build_context(&self) -> Vec<Message> {
        let mut ctx = Vec::with_capacity(self.messages.len() + 1);
        if let Some(s) = &self.summary {
            ctx.push(Message::System {
                content: format!("【以下为之前对话的摘要，请基于这些上下文继续对话】\n{s}"),
            });
        }
        ctx.extend_from_slice(&self.messages);
        ctx
    }

    /// 如果有摘要，返回一条 System Message 可插入对话开头
    pub fn summary_message(&self) -> Option<Message> {
        self.summary.as_ref().map(|s| Message::System {
            content: format!("【以下为之前对话的摘要，请基于这些上下文继续对话】\n{s}"),
        })
    }

    // ── 压缩 ──

    /// 当消息数超过 `max_messages` 时，调用 AI 模型压缩前半部分为摘要，
    /// 并保留后半部分继续对话。
    ///
    /// 返回 `true` 表示执行了压缩。
    pub async fn compact<C>(&mut self, model: &C) -> Result<bool>
    where
        C: CompletionModel,
    {
        if self.messages.len() <= self.max_messages {
            return Ok(false);
        }

        let split_at = self.messages.len() / 2;
        let old_messages: Vec<Message> = self.messages.drain(..split_at).collect();
        let conversation_text = format_messages_for_summary(&old_messages);

        let summary_prompt = format!(
            "请用中文简洁地总结以下对话的关键信息和重要上下文。只输出摘要本身，不要添加额外说明。\n\n{conversation_text}"
        );

        tracing::info!(
            "记忆压缩: {} 条消息 → 摘要 (保留 {} 条)",
            old_messages.len(),
            self.messages.len()
        );

        match model.completion_request(&summary_prompt).send().await {
            Ok(response) => {
                let choice = response.choice.first();
                let text = match &choice {
                    AssistantContent::Text(t) => t.text.clone(),
                    _ => String::new(),
                };
                if !text.is_empty() {
                    let new_summary = if let Some(ref prev) = self.summary {
                        format!("{prev}\n---\n{text}")
                    } else {
                        text
                    };
                    self.summary = Some(new_summary);
                    tracing::info!(
                        "记忆压缩完成，摘要长度: {} 字符",
                        self.summary.as_ref().map_or(0, |s| s.len())
                    );
                }
            }
            Err(e) => {
                tracing::warn!("记忆压缩失败，回退到直接截断: {e}");
            }
        }

        Ok(true)
    }

    // ── 持久化 ──

    /// 保存当前消息和摘要到 JSON 文件
    pub fn save_to_disk(&self) -> Result<()> {
        save_history(&self.messages, self.summary.as_deref())
    }

    /// 从 JSON 文件加载历史消息，返回摘要文本
    pub fn load_from_disk() -> Result<(Vec<Message>, Option<String>)> {
        load_history()
    }
}

// ============================================================
// 摘要辅助
// ============================================================

/// 将消息列表格式化为适合送给 AI 做摘要的文本
fn format_messages_for_summary(messages: &[Message]) -> String {
    messages
        .iter()
        .filter_map(|msg| match msg {
            Message::User { content } => {
                let text: String = content
                    .iter()
                    .filter_map(|c| match c {
                        UserContent::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.is_empty() {
                    None
                } else {
                    Some(format!("User: {text}"))
                }
            }
            Message::Assistant { content, .. } => {
                let text: String = content
                    .iter()
                    .filter_map(|c| match c {
                        AssistantContent::Text(t) => Some(t.text.clone()),
                        AssistantContent::Reasoning(r) => Some(r.display_text()),
                        AssistantContent::ToolCall(tc) => Some(tc.function.name.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.is_empty() {
                    None
                } else {
                    Some(format!("Assistant: {text}"))
                }
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

// ============================================================
// 消息文本提取（供 /history 命令使用）
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
                _ => None,
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
pub fn save_history(messages: &[Message], summary: Option<&str>) -> Result<()> {
    let path = config::history_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[derive(serde::Serialize)]
    struct HistoryFile<'a> {
        summary: Option<&'a str>,
        messages: &'a [Message],
    }
    let file = HistoryFile { summary, messages };
    let json = serde_json::to_string_pretty(&file)?;
    std::fs::write(&path, json)?;
    tracing::debug!(
        "对话历史已保存到: {} ({} 条消息)",
        path.display(),
        messages.len()
    );
    Ok(())
}

/// 从 JSON 文件加载对话历史和摘要
fn load_history() -> Result<(Vec<Message>, Option<String>)> {
    let path = config::history_path();
    if !path.exists() {
        return Ok((vec![], None));
    }
    let json = std::fs::read_to_string(&path)?;

    #[derive(serde::Deserialize)]
    struct HistoryFile {
        summary: Option<String>,
        messages: Vec<Message>,
    }

    // 尝试新格式，失败则回退到旧格式（纯消息数组）
    let history: HistoryFile = serde_json::from_str(&json).unwrap_or_else(|_| HistoryFile {
        summary: None,
        messages: serde_json::from_str(&json).unwrap_or_default(),
    });

    tracing::debug!(
        "从 {} 加载了 {} 条历史消息, 摘要: {}",
        path.display(),
        history.messages.len(),
        if history.summary.is_some() {
            "有"
        } else {
            "无"
        }
    );
    Ok((history.messages, history.summary))
}
