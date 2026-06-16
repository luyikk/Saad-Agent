//! 内置斜杠命令处理器
//!
//! 处理用户在交互界面中输入的 `/` 前缀命令。

use console::style;
use rig::message::Message;

use crate::config;
use crate::memory;
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
pub fn handle_command(cmd: &str, history: &mut Vec<Message>, max_history: usize) -> CommandResult {
    let cmd_lower = cmd.to_lowercase();

    // 匹配退出命令
    if cmd_lower == "/exit" || cmd_lower == "/quit" {
        if !history.is_empty() {
            let _ = memory::save_history(history, None);
        }
        ui::print_goodbye(!history.is_empty());
        return CommandResult::Exit;
    }

    // 匹配 /effort 命令（支持带参数的子命令）
    if cmd_lower.starts_with("/effort") {
        return handle_effort(&cmd_lower);
    }

    // 精确匹配其他命令
    match cmd_lower.as_str() {
        "/help" => {
            ui::print_help();
            CommandResult::Continue
        }
        "/clear" => {
            history.clear();
            let _ = std::fs::remove_file(config::history_path());
            ui::print_success("对话历史已清空");
            CommandResult::Continue
        }
        "/save" => {
            if history.is_empty() {
                ui::print_warning("对话历史为空，无需保存");
            } else {
                match memory::save_history(history, None) {
                    Ok(()) => {
                        ui::print_success(&format!("对话历史已保存 ({} 条消息)", history.len()));
                    }
                    Err(e) => ui::print_error(&format!("保存失败: {e}")),
                }
            }
            CommandResult::Continue
        }
        "/load" => {
            match memory::ConversationMemory::load_from_disk() {
                Ok((messages, _summary)) => {
                    if messages.is_empty() {
                        ui::print_warning("没有找到保存的对话历史");
                    } else {
                        let count = messages.len();
                        *history = messages;
                        ui::print_success(&format!("已加载 {count} 条历史消息"));
                    }
                }
                Err(e) => ui::print_error(&format!("加载失败: {e}")),
            }
            CommandResult::Continue
        }
        "/history" => {
            if history.is_empty() {
                ui::print_info("当前对话历史为空");
            } else {
                println!(
                    "{} 当前对话历史: {} 条消息 (限制: {} 条)",
                    ui::s_dim("📝"),
                    style(history.len()).yellow(),
                    max_history
                );
                let start = if history.len() > 5 {
                    history.len() - 5
                } else {
                    0
                };
                ui::print_divider();
                for (i, msg) in history.iter().enumerate().skip(start) {
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
            CommandResult::Continue
        }
        _ => {
            ui::print_error(&format!("未知命令: {cmd}。输入 /help 查看可用命令"));
            CommandResult::Continue
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
