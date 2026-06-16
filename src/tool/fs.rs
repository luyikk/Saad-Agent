/// 文件操作工具
///
/// 提供安全的文件读写功能，所有写操作需要用户权限确认。
/// 内置路径穿越防护，检测到越权访问时弹框让用户确认。
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing;

use crate::error::AgentError;
use crate::permission;

// ============================================================
// 路径安全校验
// ============================================================

/// 将用户提供的路径解析为安全的工作目录内的绝对路径。
///
/// # 安全保证
/// - 相对路径基于当前工作目录解析
/// - 绝对路径也必须在当前工作目录子树内（防止 `/etc/passwd` 这类逃逸）
/// - 符号链接解析后的真实路径也会被检查
///
/// # 用户交互
/// - 检测到路径逃逸时，弹出确认对话框让用户选择是否继续
/// - 用户同意后放行，拒绝则返回错误
///
/// # 错误
/// - 路径不存在时无法 canonicalize
/// - 用户拒绝了越权访问
fn resolve_safe_path(requested: &str) -> Result<PathBuf, AgentError> {
    let cwd =
        std::env::current_dir().map_err(|e| AgentError::Other(format!("无法获取当前目录: {e}")))?;

    let cwd_clean = cwd
        .canonicalize()
        .map_err(|e| AgentError::Other(format!("无法规范化当前目录: {e}")))?;

    let candidate = cwd.join(requested);

    // 尝试 canonicalize（路径必须存在）
    let resolved = candidate.canonicalize().map_err(|e| {
        AgentError::Other(format!(
            "无法解析路径 '{requested}': {e}\n   提示: 请使用已存在的路径，或使用 WriteFile 创建新文件"
        ))
    })?;

    // 安全检查：必须在当前工作目录子树内
    if !resolved.starts_with(&cwd_clean) {
        let msg = format!(
            "路径逃逸到工作目录之外\n   请求路径: {requested}\n   解析路径: {}\n   工作目录: {}",
            resolved.display(),
            cwd_clean.display()
        );
        permission::confirm_cross_directory(&msg)?;
    }

    Ok(resolved)
}

/// 为写入操作解析路径（允许目标文件尚不存在，但父目录必须在工作目录内）
fn resolve_safe_path_for_write(requested: &str) -> Result<PathBuf, AgentError> {
    let cwd =
        std::env::current_dir().map_err(|e| AgentError::Other(format!("无法获取当前目录: {e}")))?;

    let cwd_clean = cwd
        .canonicalize()
        .map_err(|e| AgentError::Other(format!("无法规范化当前目录: {e}")))?;

    let candidate = cwd.join(requested);

    // 如果文件已存在，直接 canonicalize 并检查
    if candidate.exists() {
        let resolved = candidate
            .canonicalize()
            .map_err(|e| AgentError::Other(format!("无法解析路径 '{requested}': {e}")))?;
        if !resolved.starts_with(&cwd_clean) {
            let msg = format!(
                "路径逃逸到工作目录之外\n   请求路径: {requested}\n   工作目录: {}",
                cwd_clean.display()
            );
            permission::confirm_cross_directory(&msg)?;
        }
        return Ok(resolved);
    }

    // 文件不存在：沿父目录向上找第一个存在的祖先，验证其在工作目录内
    let mut check = candidate.clone();
    loop {
        match check.parent() {
            Some(parent) if parent.as_os_str().is_empty() => {
                // 到达根目录，说明路径完全不存在
                // 回退：检查 candidate 本身（未经 canonicalize）是否以 cwd_clean 开头
                if !candidate.starts_with(&cwd_clean) {
                    let msg = format!(
                        "路径逃逸到工作目录之外\n   请求路径: {requested}\n   工作目录: {}",
                        cwd_clean.display()
                    );
                    permission::confirm_cross_directory(&msg)?;
                }
                return Ok(candidate);
            }
            Some(parent) => {
                if parent.exists() {
                    let resolved_parent = parent
                        .canonicalize()
                        .map_err(|e| AgentError::Other(format!("无法解析父目录: {e}")))?;
                    if !resolved_parent.starts_with(&cwd_clean) {
                        let msg = format!(
                            "路径逃逸到工作目录之外\n   请求路径: {requested}\n   工作目录: {}",
                            cwd_clean.display()
                        );
                        permission::confirm_cross_directory(&msg)?;
                    }
                    // 父目录安全，在原 candidate 基础上拼接剩余部分
                    let remainder = candidate.strip_prefix(parent).unwrap_or(Path::new(""));
                    return Ok(resolved_parent.join(remainder));
                }
                check = parent.to_path_buf();
            }
            None => {
                return Ok(candidate);
            }
        }
    }
}

// ============================================================
// ReadFile
// ============================================================

#[derive(Deserialize, Debug)]
pub struct ReadFileArgs {
    /// 要读取的文件路径
    pub path: String,
    /// 可选：读取的最大行数（默认 500）
    pub max_lines: Option<usize>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ReadFile;

impl Tool for ReadFile {
    const NAME: &'static str = "ReadFile";

    type Error = AgentError;
    type Args = ReadFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "ReadFile".to_string(),
            description: "读取指定路径的文件内容。支持文本文件，返回带行号的内容。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "要读取的文件完整路径"
                    },
                    "max_lines": {
                        "type": "integer",
                        "description": "读取的最大行数，默认 500"
                    }
                },
                "required": ["path"],
            }),
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let max_lines = args.max_lines.unwrap_or(500);

        // 路径安全校验（检测到越权会弹框确认）
        let safe_path = resolve_safe_path(&args.path)?;

        tracing::trace!("读取文件: {}，最大行数: {max_lines}", safe_path.display());

        let content = tokio::fs::read_to_string(&safe_path).await.map_err(|e| {
            AgentError::Other(format!("无法读取文件 '{}': {e}", safe_path.display()))
        })?;

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();

        // 截断到 max_lines
        let display_lines: Vec<&str> = if lines.len() > max_lines {
            lines.into_iter().take(max_lines).collect()
        } else {
            lines
        };

        // 带行号输出
        let numbered: Vec<String> = display_lines
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>6} | {}", i + 1, line))
            .collect();

        let mut result = numbered.join("\n");

        if total > max_lines {
            let _ = write!(
                result,
                "\n\n... (已截断，共 {total} 行，只显示前 {max_lines} 行)"
            );
        }

        Ok(result)
    }
}

// ============================================================
// WriteFile
// ============================================================

#[derive(Deserialize, Debug)]
pub struct WriteFileArgs {
    /// 要写入的文件路径
    pub path: String,
    /// 要写入的内容
    pub content: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct WriteFile;

impl Tool for WriteFile {
    const NAME: &'static str = "WriteFile";

    type Error = AgentError;
    type Args = WriteFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "WriteFile".to_string(),
            description: "将内容写入指定路径的文件。⚠️ 会覆盖已有文件！需要用户确认后才能执行。"
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "要写入的文件完整路径"
                    },
                    "content": {
                        "type": "string",
                        "description": "要写入的文件内容"
                    }
                },
                "required": ["path", "content"],
            }),
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // 文件写入需要用户确认
        crate::permission::confirm_file_write(&args.path)?;

        // 路径安全校验（检测到越权会弹框确认）
        let safe_path = resolve_safe_path_for_write(&args.path)?;

        // 确保父目录存在
        if let Some(parent) = safe_path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    AgentError::Other(format!("无法创建目录 '{}': {}", parent.display(), e))
                })?;
            }
        }

        tracing::trace!(
            "写入文件: {}，{} 字节",
            safe_path.display(),
            args.content.len()
        );

        tokio::fs::write(&safe_path, &args.content)
            .await
            .map_err(|e| {
                AgentError::Other(format!("无法写入文件 '{}': {e}", safe_path.display()))
            })?;

        Ok(format!(
            "✅ 已成功写入文件: {} ({} 字节)",
            safe_path.display(),
            args.content.len()
        ))
    }
}
