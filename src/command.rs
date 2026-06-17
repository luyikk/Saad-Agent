//! 内置斜杠命令处理器
//!
//! 处理用户在交互界面中输入的 `/` 前缀命令。

use console::style;
use sqlx::SqlitePool;

use crate::config;
use crate::memory;
use crate::memory::ConversationMemory;
use crate::session;
use crate::ui;

/// 命令处理结果
pub enum CommandResult {
    /// 继续运行
    Continue,
    /// 退出程序
    Exit,
    /// 需要重建 Agent（effort level 改变）
    RebuildAgent,
}

/// 处理内置斜杠命令。
pub async fn handle_command(
    cmd: &str,
    memory: &mut ConversationMemory,
    max_history: usize,
    pool: &SqlitePool,
) -> anyhow::Result<CommandResult> {
    let cmd_lower = cmd.to_lowercase();
    // 匹配退出命令
    if cmd_lower == "/exit" || cmd_lower == "/quit" {
        ui::print_goodbye(!memory.is_empty());
        return Ok(CommandResult::Exit);
    }

    // 匹配 /effort 命令（支持带参数的子命令）
    if cmd_lower.starts_with("/effort") {
        return Ok(handle_effort(&cmd_lower));
    }

    // 精确匹配其他命令
    match cmd_lower.as_str() {
        "/help" => {
            ui::print_help();
            Ok(CommandResult::Continue)
        }
        "/clear" => {
            let sid = memory.session_id.clone();
            memory.clear();
            // 从 DB 中删除（文件不存在时不报错）
            if let Err(e) = session::delete(pool, &sid).await {
                tracing::warn!("删除 session 文件失败: {e}");
            }
            ui::print_success("对话历史已清空");
            Ok(CommandResult::Continue)
        }
        "/save" => {
            if memory.is_empty() {
                ui::print_warning("对话历史为空，无需保存");
            } else {
                let messages_json = serde_json::to_string(memory.messages())
                    .map_err(|e| anyhow::anyhow!("序列化失败: {e}"))?;
                session::save(
                    pool,
                    &memory.session_id,
                    &memory.title,
                    memory.summary(),
                    &messages_json,
                )
                .await?;
                ui::print_success("对话历史已保存");
            }
            Ok(CommandResult::Continue)
        }
        "/load" => {
            match session::list_all(pool).await {
                Ok(sessions) if sessions.is_empty() => {
                    ui::print_warning("没有找到保存的对话历史");
                }
                Ok(sessions) => {
                    let items: Vec<String> = sessions
                        .iter()
                        .map(|s| format!("{} — {} ({} 条消息)", s.id, s.title, s.msg_count))
                        .collect();

                    let selection =
                        dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
                            .with_prompt("选择要加载的对话")
                            .items(&items)
                            .default(0)
                            .interact()
                            .map_err(|e| anyhow::anyhow!("交互选择失败: {e}"))?;

                    let meta = &sessions[selection];
                    let (messages, _summary, title) = session::load(pool, &meta.id).await?;
                    *memory.messages_mut() = messages;
                    memory.session_id = meta.id.clone();
                    memory.title = title.clone();
                    // 恢复 summary（通过 clear + 重建 summary）
                    // 注意：ConversationMemory 没有直接设置 summary 的方法，
                    // 但我们通过 from_parts 的机制来重建。这里简单替换 messages。
                    let count = memory.len();
                    ui::print_success(&format!(
                        "已加载 session: {}\n   {} ({} 条消息)",
                        meta.id, title, count
                    ));
                }
                Err(e) => ui::print_error(&format!("加载失败: {e}")),
            }
            Ok(CommandResult::Continue)
        }
        "/history" => {
            if memory.is_empty() {
                ui::print_info("当前对话历史为空");
            } else {
                println!(
                    "{} Session: {}",
                    ui::s_dim("📝"),
                    style(&memory.session_id).cyan()
                );
                println!("   标题: {}", style(&memory.title).dim());
                println!(
                    "   消息: {} 条 (限制: {} 条)",
                    style(memory.len()).yellow(),
                    max_history
                );
                let start = if memory.len() > 5 {
                    memory.len() - 5
                } else {
                    0
                };
                ui::print_divider();
                for (i, msg) in memory.messages().iter().enumerate().skip(start) {
                    let role = memory::message_role_name(msg);
                    let role_styled = match role {
                        "user" => style("user").cyan(),
                        "assistant" => style("assistant").green(),
                        "system" => style("system").dim(),
                        _ => style(role),
                    };
                    let preview = memory::message_preview(msg, 70);
                    println!(
                        "  [{}] {} {}",
                        style(i).dim(),
                        role_styled,
                        style(preview).dim()
                    );
                }
                ui::print_divider();
            }
            Ok(CommandResult::Continue)
        }
        _ => {
            ui::print_error(&format!("未知命令: {cmd}。输入 /help 查看可用命令"));
            Ok(CommandResult::Continue)
        }
    }
}

/// 处理 `/effort` 命令
///
/// 用法:
/// - `/effort`           — 显示当前 effort level
/// - `/effort concise`   — 切换为精炼模式
/// - `/effort normal`    — 切换为正常模式
/// - `/effort elaborate` — 切换为详细模式
fn handle_effort(cmd: &str) -> CommandResult {
    // 提取参数: "/effort" 或 "/effort concise"
    let arg = cmd
        .strip_prefix("/effort")
        .unwrap_or("")
        .trim()
        .to_lowercase();

    if arg.is_empty() {
        // 仅显示当前 level
        let current = config::get_effort_level();
        println!(
            "{} 当前努力程度: {}",
            ui::s_dim("🎯"),
            style(current.display_name()).cyan().bold()
        );
        println!(
            "  {} /effort {}",
            ui::s_dim("💡 可用值:"),
            ui::s_dim("concise | normal | elaborate")
        );
        return CommandResult::Continue;
    }

    match config::EffortLevel::from_str(&arg) {
        Some(level) => {
            let current = config::get_effort_level();
            if level == current {
                ui::print_info(&format!("努力程度已是 {}，无需更改", level.display_name()));
                CommandResult::Continue
            } else {
                config::set_dynamic_effort(level);
                ui::print_success(&format!("努力程度已切换为: {}", level.display_name()));
                println!("  {} Agent 将在下一轮对话时使用新设置重建", ui::s_dim("🔄"));
                CommandResult::RebuildAgent
            }
        }
        None => {
            ui::print_error(&format!(
                "无效的努力程度: \"{arg}\"。有效值: concise, normal, elaborate"
            ));
            CommandResult::Continue
        }
    }
}
