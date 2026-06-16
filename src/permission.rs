/// 权限管理系统
///
/// 控制 AI Agent 执行命令和文件操作时的用户授权策略。
/// 支持四种级别：每次询问、会话内全部允许、永久允许、拒绝。
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::io::AsyncBufReadExt;

use crate::tool::cmd::CmdError;
use crate::tool::fs::FsError;

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

/// 显示权限选择菜单并获取用户选择
async fn prompt_user(action_desc: &str, detail: &str) -> Result<char, std::io::Error> {
    println!();
    println!("╔══════════════════════════════════════════════════╗");
    println!("║  ⚠️  {action_desc}",);
    println!("║     🔧 {detail}",);
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
        .await?;

    Ok(confirmation
        .trim()
        .to_lowercase()
        .chars()
        .next()
        .unwrap_or('n'))
}

/// 处理用户选择结果
fn handle_choice(choice: char, action_desc: &str) -> Result<(), String> {
    match choice {
        'y' => Ok(()),
        'a' => {
            PERMISSION_LEVEL.store(PERM_SESSION_ALLOW_ALL, Ordering::Relaxed);
            println!("✅ 本次会话中所有操作将自动允许执行。");
            Ok(())
        }
        'p' => {
            PERMISSION_LEVEL.store(PERM_PERMANENT_ALLOW_ALL, Ordering::Relaxed);
            save_permanent_permission();
            println!(
                "✅ 已永久允许。如需恢复询问，请删除文件: {}",
                crate::config::perm_config_path().display()
            );
            Ok(())
        }
        _ => Err(format!("用户拒绝了操作: {action_desc}")),
    }
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

    match prompt_user("即将执行命令:", cmdline).await {
        Ok(choice) => {
            handle_choice(choice, &format!("命令执行: {cmdline}")).map_err(CmdError::StringError)
        }
        Err(e) => Err(CmdError::StdError(e)),
    }
}

/// 询问用户是否允许写入文件
pub async fn confirm_file_write(path: &str) -> Result<(), FsError> {
    let level = PERMISSION_LEVEL.load(Ordering::Relaxed);
    match level {
        PERM_SESSION_ALLOW_ALL | PERM_PERMANENT_ALLOW_ALL => return Ok(()),
        _ => {}
    }

    match prompt_user("即将写入文件:", path).await {
        Ok(choice) => {
            handle_choice(choice, &format!("文件写入: {path}")).map_err(|e| FsError::Other(e))
        }
        Err(e) => Err(FsError::Io(e)),
    }
}
