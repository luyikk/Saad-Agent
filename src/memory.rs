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

/// 单条工具结果/消息文本的最大字符数（超过则截断以节省 token）
const MAX_MESSAGE_CHARS: usize = 4000;

impl ConversationMemory {
    /// 创建新的记忆体
    #[allow(dead_code)]
    pub fn new(max_messages: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_messages,
            summary: None,
        }
    }

    /// 从已有消息列表创建（用于加载历史）
    pub fn from_parts(
        messages: Vec<Message>,
        summary: Option<String>,
        max_messages: usize,
    ) -> Self {
        Self {
            messages,
            max_messages,
            summary,
        }
    }

    // ── 基本访问 ──

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.messages.clear();
        self.summary = None;
    }

    /// 扩展消息历史，只保留纯文本聊天记录：
    ///
    /// 1. **只留 Text** — User 只保留 `UserContent::Text`，Assistant 只保留 `AssistantContent::Text`
    /// 2. **空消息** — 丢弃过滤后没有任何文本内容的消息
    /// 3. **过长截断** — 单条消息超过 `MAX_MESSAGE_CHARS` 字符时自动截断
    /// 4. **连续重复** — 丢弃与上一条完全相同的消息（常见于重试/循环）
    pub fn extend(&mut self, new_messages: &[Message]) {
        for msg in new_messages {
            // ① 只保留 Text 内容，丢弃 ToolCall/ToolResult/Reasoning/Image 等
            let filtered = keep_only_text(msg);

            // ② 跳过过滤后为空的消息
            let Some(mut filtered) = filtered else {
                tracing::debug!("跳过空消息(无 Text 内容): role={}", message_role_name(msg));
                continue;
            };

            // ③ 截断过长文本
            truncate_message_texts(&mut filtered);

            // ④ 去重：跳过与上一条完全相同的消息
            if self.messages.last() == Some(&filtered) {
                tracing::debug!("跳过重复消息: role={}", message_role_name(msg));
                continue;
            }

            self.messages.push(filtered);
        }
    }

    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub fn summary_message(&self) -> Option<Message> {
        self.summary.as_ref().map(|s| Message::System {
            content: format!("【以下为之前对话的摘要，请基于这些上下文继续对话】\n{s}"),
        })
    }

    // ── 压缩 ──

    /// 当消息数超过 `max_messages` 时，调用 AI 模型压缩前半部分为摘要，
    /// 并保留后半部分继续对话。
    ///
    /// **安全保证**：先完成 AI 摘要调用，成功后才从内存中移除旧消息。
    /// 若 AI 调用失败，消息完整保留，不会丢失数据。
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

        // 【修复】先克隆旧消息用于生成摘要，不立即 drain。
        // 只有 AI 调用成功后才从 Vec 中移除，防止网络/API 故障导致数据丢失。
        let old_messages: Vec<Message> = self.messages[..split_at].to_vec();
        let conversation_text = format_messages_for_summary(&old_messages);

        let summary_prompt = format!(
            "请用中文简洁地总结以下对话的关键信息和重要上下文。只输出摘要本身，不要添加额外说明。\n\n{conversation_text}"
        );

        tracing::info!(
            "记忆压缩: {} 条消息 → 摘要 (保留 {} 条，总计 {} 条)",
            old_messages.len(),
            self.messages.len() - old_messages.len(),
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

                // ✅ 摘要成功后，安全移除旧消息
                self.messages.drain(..split_at);
                self.save_to_disk()?;
                Ok(true)
            }
            Err(e) => {
                // ✅ 摘要失败，消息完整保留，回退到截断策略
                tracing::warn!(
                    "记忆压缩失败，回退到直接截断 (保留最近 {} 条): {e}",
                    self.max_messages
                );
                // 截断到 max_messages 以内（保留最新的消息）
                if self.messages.len() > self.max_messages {
                    let excess = self.messages.len() - self.max_messages;
                    self.messages.drain(..excess);
                }
                Ok(true)
            }
        }
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
// extend 辅助: 剥离工具内容
// ============================================================

// ============================================================
// extend 辅助: 消息过滤与截断
// ============================================================

/// 只保留 Text 内容：User 只留 `UserContent::Text`，Assistant 只留 `AssistantContent::Text`
///
/// 丢弃：ToolCall、ToolResult、Reasoning、Image 等所有非纯文本内容。
/// 过滤后无文本返回 `None`，表示整条消息应丢弃。
fn keep_only_text(msg: &Message) -> Option<Message> {
    match msg {
        Message::User { content } => {
            let texts: Vec<String> = content
                .iter()
                .filter_map(|c| match c {
                    UserContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(Message::User {
                    content: rig::one_or_many::OneOrMany::one(UserContent::text(texts.join("\n"))),
                })
            }
        }
        Message::Assistant { content, .. } => {
            let texts: Vec<String> = content
                .iter()
                .filter_map(|c| match c {
                    AssistantContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(Message::Assistant {
                    id: None,
                    content: rig::one_or_many::OneOrMany::one(AssistantContent::text(
                        texts.join("\n"),
                    )),
                })
            }
        }
        Message::System { .. } => Some(msg.clone()),
    }
}

/// 就地截断消息中超过 `MAX_MESSAGE_CHARS` 的文本字段
fn truncate_message_texts(msg: &mut Message) {
    match msg {
        Message::System { content } => {
            truncate_str(content, "系统消息");
        }
        Message::User { content } => {
            for c in content.iter_mut() {
                if let UserContent::Text(t) = c {
                    truncate_str(&mut t.text, "文本");
                }
            }
        }
        Message::Assistant { content, .. } => {
            for c in content.iter_mut() {
                if let AssistantContent::Text(t) = c {
                    truncate_str(&mut t.text, "助手回复");
                }
            }
        }
    }
}

/// 如果字符串超过 `MAX_MESSAGE_CHARS` 则截断并附加提示
fn truncate_str(s: &mut String, label: &str) {
    if s.chars().count() > MAX_MESSAGE_CHARS {
        let truncated: String = s.chars().take(MAX_MESSAGE_CHARS).collect();
        *s = format!("{truncated}\n\n[{label}过长，已截断，保留前 {MAX_MESSAGE_CHARS} 字符]");
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
    match serde_json::from_str::<HistoryFile>(&json) {
        Ok(h) => {
            tracing::debug!(
                "从 {} 加载了 {} 条历史消息",
                path.display(),
                h.messages.len()
            );
            Ok((h.messages, h.summary))
        }
        Err(_) => {
            let messages: Vec<Message> = serde_json::from_str(&json)?;
            tracing::debug!(
                "从 {} 加载了 {} 条历史消息（旧格式）",
                path.display(),
                messages.len()
            );
            Ok((messages, None))
        }
    }
}
