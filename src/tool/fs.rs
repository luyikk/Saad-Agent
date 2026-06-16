/// 文件操作工具
///
/// 提供安全的文件读写功能，所有写操作需要用户权限确认。
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use tracing;

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

#[derive(Debug, Error)]
pub enum FsError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ReadFile;

impl Tool for ReadFile {
    const NAME: &'static str = "ReadFile";

    type Error = FsError;
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
        let path = &args.path;
        let max_lines = args.max_lines.unwrap_or(500);

        tracing::trace!("读取文件: {path}，最大行数: {max_lines}");

        let content = std::fs::read_to_string(path)
            .map_err(|e| FsError::Other(format!("无法读取文件 '{path}': {e}")))?;

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
            result.push_str(&format!(
                "\n\n... (已截断，共 {total} 行，只显示前 {max_lines} 行)"
            ));
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

    type Error = FsError;
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
        let path = &args.path;
        let content = &args.content;

        // 文件写入需要用户确认
        crate::permission::confirm_file_write(path).await?;

        // 确保父目录存在
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    FsError::Other(format!("无法创建目录 '{}': {}", parent.display(), e))
                })?;
            }
        }

        tracing::trace!("写入文件: {path}，{} 字节", content.len());

        std::fs::write(path, content)
            .map_err(|e| FsError::Other(format!("无法写入文件 '{path}': {e}")))?;

        Ok(format!(
            "✅ 已成功写入文件: {path} ({} 字节)",
            content.len()
        ))
    }
}
