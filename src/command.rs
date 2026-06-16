//! 内置斜杠命令处理器
//!
//! 处理用户在交互界面中输入的 `/` 前缀命令。

use console::style;
use rig::message::Message;

use crate::config;
use crate::history;
use crate::ui;

/// 处理内置斜杠命令。
///
/// 返回值：
/// - `Some(true)` — 应退出程序
/// - `None`        — 继续运行
pub async fn handle_command(
    cmd: &str,
    history: &mut Vec<Message>,
    max_history: usize,
) -> Option<bool> {
    match cmd.to_lowercase().as_str() {
        "/exit" | "/quit" => {
            if !history.is_empty() {
                let _ = history::save_history(history);
            }
            ui::print_goodbye(!history.is_empty());
            Some(true) // 退出
        }
        "/help" => {
            ui::print_help();
            None
        }
        "/clear" => {
            history.clear();
            let _ = std::fs::remove_file(config::history_path());
            ui::print_success("对话历史已清空");
            None
        }
        "/save" => {
            if history.is_empty() {
                ui::print_warning("对话历史为空，无需保存");
            } else {
                match history::save_history(history) {
                    Ok(()) => {
                        ui::print_success(&format!("对话历史已保存 ({} 条消息)", history.len()))
                    }
                    Err(e) => ui::print_error(&format!("保存失败: {}", e)),
                }
            }
            None
        }
        "/load" => {
            match history::load_history() {
                Ok(loaded) => {
                    if loaded.is_empty() {
                        ui::print_warning("没有找到保存的对话历史");
                    } else {
                        let count = loaded.len();
                        *history = loaded;
                        ui::print_success(&format!("已加载 {} 条历史消息", count));
                    }
                }
                Err(e) => ui::print_error(&format!("加载失败: {}", e)),
            }
            None
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
                    let role = history::message_role_name(msg);
                    let role_styled = match role {
                        "user" => style("user").cyan(),
                        "assistant" => style("assistant").green(),
                        "system" => style("system").dim(),
                        _ => style(role),
                    };
                    let preview = history::message_preview(msg, 70);
                    println!(
                        "  [{}] {} {}",
                        style(i).dim(),
                        role_styled,
                        style(preview).dim()
                    );
                }
                ui::print_divider();
            }
            None
        }
        _ => {
            ui::print_error(&format!("未知命令: {cmd}。输入 /help 查看可用命令"));
            None
        }
    }
}
