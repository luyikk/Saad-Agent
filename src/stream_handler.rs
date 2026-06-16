//! AI 流式响应处理器
//!
//! 将 `agent.stream_chat()` 返回的流数据解析为终端 UI 展示，
//! 最终返回 `FinalResponse` 供调用方更新对话历史。

use futures_util::stream::StreamExt;
use rig::agent::{CompletionCall, FinalResponse, MultiTurnStreamItem, Text};
use rig::message::{ToolCall, ToolResult, ToolResultContent};
use rig::streaming::{StreamedAssistantContent, StreamedUserContent};

use crate::ui::StreamDisplay;

/// 处理一条 AI 流式响应，驱动 `display` 渲染，返回最终的 `FinalResponse`。
///
/// # 类型参数
/// - `R` — CompletionModel 的 Response 类型（由 `stream_chat` 自动推导）
/// - `E` — 流错误类型（由 `stream_chat` 自动推导，实现了 `Display`）
pub async fn process_stream<R, E>(
    _prompt: &str,
    stream: impl futures_util::stream::Stream<Item = Result<MultiTurnStreamItem<R>, E>>,
    display: &mut StreamDisplay,
) -> FinalResponse
where
    E: std::fmt::Display,
{
    let spinner = crate::ui::new_spinner("AI 正在思考...");
    let mut stream = Box::pin(stream);
    spinner.finish_and_clear();

    let mut final_res = FinalResponse::empty();

    while let Some(content) = stream.next().await {
        match content {
            // ── 回答文本增量 ──
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                Text { text, .. },
            ))) => {
                let _ = display.on_answer(&text);
            }

            // ── 推理链增量（流式） ──
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::ReasoningDelta { reasoning, .. },
            )) => {
                let _ = display.on_reasoning_delta(&reasoning);
            }

            // ── 推理链完整块 ──
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::Reasoning(reasoning),
            )) => {
                let _ = display.on_reasoning(&reasoning.display_text());
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
                let _ = display.on_tool_call(&function.name, &args_preview);
            }

            // ── 工具调用参数增量 ──
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::ToolCallDelta { content, .. },
            )) => {
                use rig::streaming::ToolCallDeltaContent;
                match content {
                    ToolCallDeltaContent::Name(name) => {
                        let _ = display.on_tool_call_delta(&format!("[调用: {}]", name));
                    }
                    ToolCallDeltaContent::Delta(delta) => {
                        let _ = display.on_tool_call_delta(&delta);
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
                let preview = truncate_str(&summary, 200);
                let _ = display.on_tool_result(success, &preview);
            }

            // ── Provider Completion 调用详情 ──
            Ok(MultiTurnStreamItem::CompletionCall(CompletionCall {
                call_index, usage, ..
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

            // ── 其他事件（忽略） ──
            _ => {}
        }
    }

    display.finalize(&final_res.usage());
    final_res
}

/// 截断字符串到指定字符数，超出部分用 `...` 表示
fn truncate_str(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > max_chars {
        format!(
            "{}...",
            chars.into_iter().take(max_chars.saturating_sub(3)).collect::<String>()
        )
    } else {
        s.to_string()
    }
}
