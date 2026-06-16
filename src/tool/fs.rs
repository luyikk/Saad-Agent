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
    /// 可选：从第几行开始读（1-based），默认从第 1 行
    pub start_line: Option<usize>,
    /// 可选：读取的最大行数，默认 500
    pub max_lines: Option<usize>,
    /// 可选：读取文件末尾的 N 行（设置后将忽略 start_line）
    pub tail_lines: Option<usize>,
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
            description: "读取指定路径的文件内容，返回带行号的内容。支持从指定行开始、限制行数、从末尾向上读取。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "要读取的文件完整路径"
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "从第几行开始读取（1-based）。不设置则从第 1 行开始。与 tail_lines 互斥"
                    },
                    "max_lines": {
                        "type": "integer",
                        "description": "读取的最大行数，默认 500"
                    },
                    "tail_lines": {
                        "type": "integer",
                        "description": "读取文件末尾的最后 N 行。设置后将忽略 start_line，从末尾向上取 N 行"
                    }
                },
                "required": ["path"],
            }),
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let max_lines = args.max_lines.unwrap_or(500);

        // 路径安全校验
        let safe_path = resolve_safe_path(&args.path)?;

        let content = tokio::fs::read_to_string(&safe_path).await.map_err(|e| {
            AgentError::Other(format!("无法读取文件 '{}': {e}", safe_path.display()))
        })?;

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();

        // 确定读取范围
        let (display_lines, line_offset, header) = if let Some(tail) = args.tail_lines {
            // 从末尾向上取 N 行
            let take = tail.min(max_lines);
            let start = total.saturating_sub(take);
            let slice = &lines[start..];
            (
                slice.to_vec(),
                start,
                format!("…（末尾 {take} 行，共 {total} 行）"),
            )
        } else {
            // 从 start_line 开始向后取
            let start_idx = args.start_line.map_or(0, |s| s.saturating_sub(1));
            let start_idx = start_idx.min(total);
            let slice = &lines[start_idx..];
            let taken: Vec<&str> = slice.iter().take(max_lines).copied().collect();
            let shown = taken.len();
            let header = if start_idx + shown < total {
                format!(
                    "…（第 {}-{} 行，共 {total} 行）",
                    start_idx + 1,
                    start_idx + shown,
                )
            } else if start_idx > 0 {
                format!("…（第 {}-{} 行，共 {total} 行）", start_idx + 1, total)
            } else {
                String::new()
            };
            (taken, start_idx, header)
        };

        // 带行号输出
        let numbered: Vec<String> = display_lines
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>6} | {}", line_offset + i + 1, line))
            .collect();

        let mut result = numbered.join("\n");

        if !header.is_empty() {
            let _ = write!(result, "\n\n{header}");
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

// ============================================================
// EditFile
// ============================================================

#[derive(Deserialize, Debug)]
pub struct EditFileArgs {
    /// 要编辑的文件路径
    pub path: String,
    /// 要替换的原始文本（必须在文件中唯一存在）
    pub old_string: String,
    /// 替换后的新文本
    pub new_string: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct EditFile;

impl Tool for EditFile {
    const NAME: &'static str = "EditFile";

    type Error = AgentError;
    type Args = EditFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "EditFile".to_string(),
            description: r#"精确编辑文件：在文件中查找 `old_string` 并替换为 `new_string`。
【⚠️ 关键规则】
- `old_string` 必须与文件中的内容完全一致（包括空格、缩进、换行），且在文件中只能出现一次
- `old_string` 必须包含足够的上下文（前后各 2-3 行）以确保唯一匹配
- 如果要删除内容，将 `new_string` 设为空字符串 ""
- 不要传整个文件内容，只传需要修改的最小片段"#
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "要编辑的文件完整路径"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "要替换的原始文本片段，必须与文件中对应内容完全一致且在文件中唯一出现。请包含足够的上下文行以确保唯一性。"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "替换后的新文本内容"
                    }
                },
                "required": ["path", "old_string", "new_string"],
            }),
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // 文件编辑需要用户确认
        crate::permission::confirm_file_write(&args.path)?;

        // 路径安全校验
        let safe_path = resolve_safe_path(&args.path)?;

        tracing::trace!(
            "编辑文件: {}，old: {} 字节，new: {} 字节",
            safe_path.display(),
            args.old_string.len(),
            args.new_string.len()
        );

        let content = tokio::fs::read_to_string(&safe_path).await.map_err(|e| {
            AgentError::Other(format!("无法读取文件 '{}': {e}", safe_path.display()))
        })?;

        // 查找 old_string 并验证唯一性
        let matches: Vec<usize> = content
            .match_indices(&args.old_string)
            .map(|(i, _)| i)
            .collect();

        match matches.len() {
            0 => {
                return Err(AgentError::Other(format!(
                    "❌ 未在文件 '{}' 中找到匹配的 old_string。\n\
                     提示:\n  \
                     - 检查文本是否完全一致（包括空白字符、缩进）\n  \
                     - old_string 必须包含足够的上下文以确保唯一性\n  \
                     - 先用 ReadFile 查看文件内容确认要修改的部分",
                    safe_path.display()
                )));
            }
            1 => {} // 唯一匹配，继续
            n => {
                // 展示冲突位置以帮助 AI 修正
                let mut hints = Vec::new();
                for (i, &pos) in matches.iter().take(5).enumerate() {
                    let start = pos.saturating_sub(40);
                    let end = (pos + args.old_string.len() + 40).min(content.len());
                    let snippet = &content[start..end];
                    hints.push(format!("  [{}] ...{}...", i + 1, snippet));
                }
                return Err(AgentError::Other(format!(
                    "❌ old_string 在文件中出现了 {n} 次（必须唯一）。\n\
                     前 5 处匹配位置:\n{}\n\
                     提示: 请包含更多上下文行（前后 2-3 行）以确保 old_string 唯一匹配。",
                    hints.join("\n")
                )));
            }
        }

        // 执行替换
        let new_content = content.replacen(&args.old_string, &args.new_string, 1);

        // 写入文件
        tokio::fs::write(&safe_path, &new_content)
            .await
            .map_err(|e| {
                AgentError::Other(format!("无法写入文件 '{}': {e}", safe_path.display()))
            })?;

        let changed = (args.new_string.len() as i64) - (args.old_string.len() as i64);
        let summary = if changed >= 0 {
            format!("+{} 字节", changed)
        } else {
            format!("{} 字节", changed)
        };

        Ok(format!(
            "✅ 已成功编辑文件: {}\n   替换: {} 字节 → {} 字节 ({})",
            safe_path.display(),
            args.old_string.len(),
            args.new_string.len(),
            summary
        ))
    }
}

// ============================================================
// GetFileLines
// ============================================================

#[derive(Deserialize, Debug)]
pub struct GetFileLinesArgs {
    /// 文件路径
    pub path: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct GetFileLines;

impl Tool for GetFileLines {
    const NAME: &'static str = "GetFileLines";

    type Error = AgentError;
    type Args = GetFileLinesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "GetFileLines".to_string(),
            description: "获取文本文件的总行数。用于在读取大文件前了解文件规模，方便决定 ReadFile 的 max_lines 参数。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "要查询的文件完整路径"
                    }
                },
                "required": ["path"],
            }),
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let safe_path = resolve_safe_path(&args.path)?;

        let content = tokio::fs::read_to_string(&safe_path).await.map_err(|e| {
            AgentError::Other(format!("无法读取文件 '{}': {e}", safe_path.display()))
        })?;

        let total = content.lines().count();

        Ok(format!("文件: {}\n总行数: {total}", safe_path.display(),))
    }
}
