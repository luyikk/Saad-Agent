/// 权限管理系统
///
/// 控制 AI Agent 执行命令时的用户授权策略。
/// 支持四种级别：每次询问、会话内全部允许、永久允许、拒绝。

use std::sync::atomic::{AtomicU8, Ordering};
use tokio::io::AsyncBufReadExt;

use crate::tool::cmd::CmdError;

/// 权限级别：每次执行都询问用户
pub const PERM_PROMPT: u8 = 0;
/// 权限级别：当前会话全部允许
pub const PERM_SESSION_ALLOW_ALL: u8 = 1;
/// 权限级别：永久允许（持久化到磁盘）
pub const PERM_PERMANENT_ALLOW_ALL: u8 = 2;

/// 全局权限状态
static PERMISSION_LEVEL: AtomicU8 = AtomicU8::new(PERM_PROMPT);

/// 从磁盘加载持久化的权限配置
pub fn load_permanent_permission() {
    let path = crate::config::perm_config_path();
    if let Ok(data) = std::fs::read_to_string(path) {
        if data.trim() == "allow_all" {
            PERMISSION_LEVEL.store(PERM_PERMANENT_ALLOW_ALL, Ordering::Relaxed);
        }
    }
}

/// 将永久允许权限保存到磁盘
fn save_permanent_permission() {
    let path = crate::config::perm_config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, "allow_all");
}

/// 询问用户是否允许执行命令
///
/// 根据当前权限级别决定是否需要交互：
/// - `PERM_SESSION_ALLOW_ALL` / `PERM_PERMANENT_ALLOW_ALL` → 自动允许
/// - `PERM_PROMPT` → 显示交互界面让用户选择
pub async fn confirm_execution(cmdline: &str) -> Result<(), CmdError> {
    let level = PERMISSION_LEVEL.load(Ordering::Relaxed);
    match level {
        PERM_SESSION_ALLOW_ALL | PERM_PERMANENT_ALLOW_ALL => return Ok(()),
        _ => {}
    }

    println!();
    println!("╔══════════════════════════════════════════════════╗");
    println!("║  ⚠️  即将执行命令:                          ║");
    println!("║     🔧 {cmdline}",);
    println!("╠══════════════════════════════════════════════════╣");
    println!("║  [y] 允许本次执行                             ║");
    println!("║  [a] 本次会话全部允许                         ║");
    println!("║  [p] 永久允许（不再询问）                     ║");
    println!("║  [N] 拒绝                                     ║");
    println!("╚══════════════════════════════════════════════════╝");
    print!("请选择 [y/a/p/N]: ");
    std::io::Write::flush(&mut std::io::stdout()).ok();

    let mut confirmation = String::new();
    tokio::io::BufReader::new(tokio::io::stdin())
        .read_line(&mut confirmation)
        .await
        .map_err(|e| CmdError::StdError(e))?;

    match confirmation.trim().to_lowercase().as_str() {
        "y" | "yes" => Ok(()),
        "a" => {
            PERMISSION_LEVEL.store(PERM_SESSION_ALLOW_ALL, Ordering::Relaxed);
            println!("✅ 本次会话中所有命令将自动允许执行。");
            Ok(())
        }
        "p" => {
            PERMISSION_LEVEL.store(PERM_PERMANENT_ALLOW_ALL, Ordering::Relaxed);
            save_permanent_permission();
            println!(
                "✅ 已永久允许。如需恢复询问，请删除文件: {}",
                crate::config::perm_config_path().display()
            );
            Ok(())
        }
        _ => Err(CmdError::StringError(format!(
            "用户拒绝了命令执行: {cmdline}"
        ))),
    }
}
