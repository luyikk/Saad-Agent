/// 命令行执行工具
///
/// 提供在本地操作系统中安全执行命令的功能，
/// 支持 Windows（PowerShell）和 Linux/macOS（sh）。
/// 在 Windows 上使用 `encoding_rs` 智能处理 GBK/UTF-8 编码转换。
use encoding_rs::GBK;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::AgentError;

#[derive(Deserialize, Debug)]
pub struct OperationArgs {
    /// 完整的命令行语句，已包含所有参数（如 "Get-ChildItem -Path d:\\"）
    pub command: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RunCmd;

impl Tool for RunCmd {
    const NAME: &'static str = "RunCmd";

    type Error = AgentError;
    type Args = OperationArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "RunCmd".to_string(),
            description:
                r#"在本地操作系统中安全地执行完整的命令行语句。支持跨平台 (Windows/Linux/macOS)。
【⚠️ 关键】command 必须是完整且可直接执行的单行命令字符串，已包含所有参数。
- ✅ Windows 正确示例：`"Get-ChildItem -Path d:\\"`、`"dir d:\\"`
- ✅ Linux/macOS 正确示例：`"ls -l /var/log"`、`"grep -r foo ./src"`
- ❌ 错误示例：不要把命令和参数分成两个字段传入。
【注意】Windows 使用 PowerShell 执行，Linux/macOS 使用 sh 执行。禁止执行破坏性或高风险的恶意命令。"#
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "完整的命令行字符串，包含程序名和所有参数。Windows 下使用 PowerShell 语法，Linux/macOS 下使用 Bash/Shell 语法。"
                    }
                },
                "required": ["command"],
            }),
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let cmdline = args.command;

        // 权限检查
        crate::permission::confirm_execution(&cmdline)?;

        tracing::trace!("正在执行命令: '{cmdline}'");

        // 【修复1】跨平台 & 跨版本 Shell 适配
        let output = if cfg!(target_os = "windows") {
            // 优先使用 pwsh (PS 7+)，回退到 powershell (5.1)
            let shell = if std::process::Command::new("pwsh")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok()
            {
                "pwsh"
            } else {
                "powershell"
            };

            std::process::Command::new(shell)
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    // PS7 和 5.1 均支持此语法设置 UTF-8
                    &format!("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; {cmdline}"),
                ])
                .output()
        } else {
            std::process::Command::new("sh")
                .args(["-c", &cmdline])
                .output()
        }
        .map_err(AgentError::Io)?;

        let stdout = decode_output(&output.stdout);
        let stderr = decode_output(&output.stderr);

        // 【修复4】成功时合并 stdout + stderr，确保 LLM 能获取完整的编译警告与进度
        let combined = format!("{}\n{}", stdout.trim(), stderr.trim())
            .trim()
            .to_string();

        if combined.is_empty() {
            Ok("命令已成功执行，无输出。".to_string())
        } else {
            Ok(combined)
        }
    }
}

/// 智能解码命令输出：优先 UTF-8，失败时在 Windows 上回退到 GBK
fn decode_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    // 优先尝试 UTF-8
    match std::str::from_utf8(bytes) {
        Ok(s) => {
            // 检查是否包含替换字符 (U+FFFD)，若有则尝试 GBK
            if s.contains('\u{FFFD}') && cfg!(target_os = "windows") {
                let (cow, _encoding, had_errors) = GBK.decode(bytes);
                if !had_errors {
                    tracing::trace!("UTF-8 解码存在替换字符，已回退到 GBK 解码");
                    return cow.into_owned();
                }
            }
            s.to_string()
        }
        Err(_) => {
            // UTF-8 解码失败，在 Windows 上使用 GBK
            if cfg!(target_os = "windows") {
                tracing::trace!("UTF-8 解码失败，回退到 GBK 解码");
                let (cow, _encoding, had_errors) = GBK.decode(bytes);
                if had_errors {
                    tracing::warn!("GBK 解码也存在错误，使用替换字符");
                }
                cow.into_owned()
            } else {
                // 非 Windows 平台使用 lossy UTF-8
                String::from_utf8_lossy(bytes).to_string()
            }
        }
    }
}
