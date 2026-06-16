/// 权限管理系统
///
/// 控制 AI Agent 执行命令和文件操作时的用户授权策略。
/// 支持四种级别：每次询问、会话内全部允许、永久允许、拒绝。
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

use crate::error::AgentError;

/// 权限级别：每次执行都询问用户
pub const PERM_PROMPT: u8 = 0;
/// 权限级别：当前会话全部允许
pub const PERM_SESSION_ALLOW_ALL: u8 = 1;
/// 权限级别：永久允许（持久化到磁盘）
pub const PERM_PERMANENT_ALLOW_ALL: u8 = 2;

/// 全局权限状态
static PERMISSION_LEVEL: AtomicU8 = AtomicU8::new(PERM_PROMPT);

// ── 持久化配置 ──

/// 权限持久化配置
#[derive(Debug, Serialize, Deserialize)]
struct PermissionConfig {
    /// 是否永久允许所有操作
    #[serde(default)]
    allow_all: bool,
    /// 配置文件版本号（用于未来兼容）
    #[serde(default = "default_version")]
    version: u32,
}

const fn default_version() -> u32 {
    1
}

impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            allow_all: false,
            version: 1,
        }
    }
}

/// 从磁盘加载持久化的权限配置
pub fn load_permanent_permission() {
    let path = crate::config::perm_config_path();
    match std::fs::read_to_string(&path) {
        Ok(data) => {
            // 优先尝试 TOML 反序列化
            match toml::from_str::<PermissionConfig>(&data) {
                Ok(config) if config.allow_all => {
                    PERMISSION_LEVEL.store(PERM_PERMANENT_ALLOW_ALL, Ordering::Relaxed);
                    tracing::info!("已加载永久允许权限配置");
                }
                Ok(_) => {
                    tracing::debug!("权限配置已加载，allow_all=false");
                }
                Err(e) => {
                    // 兼容旧格式（纯文本 "allow_all"）
                    if data.trim() == "allow_all" {
                        PERMISSION_LEVEL.store(PERM_PERMANENT_ALLOW_ALL, Ordering::Relaxed);
                        tracing::info!("已从旧格式加载永久允许权限，下次保存时将升级为 TOML");
                        // 立即升级格式
                        save_permanent_permission();
                    } else {
                        tracing::warn!("权限配置文件格式错误，将使用默认配置: {e}");
                    }
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!("权限配置文件不存在，使用默认配置");
        }
        Err(e) => {
            tracing::warn!("读取权限配置文件失败: {e}");
        }
    }
}

/// 将永久允许权限保存到磁盘（TOML 格式）
fn save_permanent_permission() {
    let path = crate::config::perm_config_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("创建权限配置目录失败: {e}");
            return;
        }
    }

    let config = PermissionConfig {
        allow_all: true,
        version: 1,
    };

    match toml::to_string_pretty(&config) {
        Ok(data) => {
            if let Err(e) = std::fs::write(&path, &data) {
                tracing::warn!("写入权限配置文件失败: {e}");
            } else {
                tracing::debug!("权限配置已保存到: {}", path.display());
            }
        }
        Err(e) => {
            tracing::warn!("序列化权限配置失败: {e}");
        }
    }
}

/// 根据用户选择处理权限变更
///
/// 返回值: `Ok(())` 表示允许执行，`Err(msg)` 表示被拒绝。
fn handle_selection(selection: usize, action_desc: &str) -> Result<(), String> {
    match selection {
        0 => {
            // 允许本次
            Ok(())
        }
        1 => {
            // 本次会话全部允许
            PERMISSION_LEVEL.store(PERM_SESSION_ALLOW_ALL, Ordering::Relaxed);
            crate::ui::print_success("本次会话中所有操作将自动允许执行。");
            Ok(())
        }
        2 => {
            // 永久允许
            PERMISSION_LEVEL.store(PERM_PERMANENT_ALLOW_ALL, Ordering::Relaxed);
            save_permanent_permission();
            crate::ui::print_success(&format!(
                "已永久允许。如需恢复询问，请删除文件: {}",
                crate::config::perm_config_path().display()
            ));
            Ok(())
        }
        _ => {
            // 拒绝
            Err(format!("用户拒绝了操作: {action_desc}"))
        }
    }
}

/// 询问用户是否允许执行命令
///
/// 根据当前权限级别决定是否需要交互：
/// - `PERM_SESSION_ALLOW_ALL` / `PERM_PERMANENT_ALLOW_ALL` → 自动允许
/// - `PERM_PROMPT` → 显示 `dialoguer::Select` 交互界面
pub fn confirm_execution(cmdline: &str) -> Result<(), AgentError> {
    let level = PERMISSION_LEVEL.load(Ordering::Relaxed);
    match level {
        PERM_SESSION_ALLOW_ALL | PERM_PERMANENT_ALLOW_ALL => return Ok(()),
        _ => {}
    }

    // 使用 dialoguer 交互选择
    crate::ui::select_permission("即将执行命令:", cmdline).map_or_else(
        || Err(AgentError::Other("权限选择已取消".to_string())),
        |selection| {
            handle_selection(selection, &format!("命令执行: {cmdline}")).map_err(AgentError::Other)
        },
    )
}

/// 询问用户是否允许写入文件
pub fn confirm_file_write(path: &str) -> Result<(), AgentError> {
    let level = PERMISSION_LEVEL.load(Ordering::Relaxed);
    match level {
        PERM_SESSION_ALLOW_ALL | PERM_PERMANENT_ALLOW_ALL => return Ok(()),
        _ => {}
    }

    crate::ui::select_permission("即将写入文件:", path).map_or_else(
        || Err(AgentError::Other("权限选择已取消".to_string())),
        |selection| {
            handle_selection(selection, &format!("文件写入: {path}")).map_err(AgentError::Other)
        },
    )
}

/// 询问用户是否允许跨目录文件访问（路径逃逸检测）
///
/// 与 `confirm_file_write` 共享同一个全局权限级别。
/// 当用户选择"会话全部允许"或"永久允许"后，路径逃逸也会自动放行。
pub fn confirm_cross_directory(detail: &str) -> Result<(), AgentError> {
    let level = PERMISSION_LEVEL.load(Ordering::Relaxed);
    match level {
        PERM_SESSION_ALLOW_ALL | PERM_PERMANENT_ALLOW_ALL => return Ok(()),
        _ => {}
    }

    let action_desc = format!(
        "安全警告：检测到路径穿越\n  {detail}",
        detail = detail.lines().next().unwrap_or(detail),
    );

    crate::ui::select_permission("跨目录文件访问:", &action_desc).map_or_else(
        || Err(AgentError::Other("权限选择已取消".to_string())),
        |selection| {
            handle_selection(selection, &format!("跨目录访问: {detail}"))
                .map_err(AgentError::Other)
        },
    )
}
