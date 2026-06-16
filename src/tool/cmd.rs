/// 命令行执行工具
///
/// 提供在本地操作系统中安全执行命令的功能，
/// 支持 Windows（PowerShell）和 Linux/macOS（sh）。

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

#[derive(Deserialize, Debug)]
pub struct OperationArgs {
    /// 完整的命令行语句，已包含所有参数（如 "Get-ChildItem -Path d:\\"）
    pub command: String,
}

#[derive(Debug, Error)]
pub enum CmdError {
    #[error("IO 错误: {0}")]
    StdError(#[from] std::io::Error),
    #[error("命令执行错误: {0}")]
    StringError(String),
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RunCmd;

impl Tool for RunCmd {
    const NAME: &'static str = "RunCmd";

    type Error = CmdError;
    type Args = OperationArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "RunCmd".to_string(),
            description: r#"在本地操作系统中安全地执行完整的命令行语句。支持跨平台 (Windows/Linux/macOS)。
【⚠️ 关键】command 必须是完整且可直接执行的单行命令字符串，已包含所有参数。
- ✅ Windows 正确示例：`"Get-ChildItem -Path d:\\"`、`"dir d:\\"`
- ✅ Linux/macOS 正确示例：`"ls -l /var/log"`、`"grep -r foo ./src"`
- ❌ 错误示例：不要把命令和参数分成两个字段传入。
【注意】Windows 使用 PowerShell 执行，Linux/macOS 使用 sh 执行。禁止执行破坏性或高风险的恶意命令。"#.to_string(),
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
        crate::permission::confirm_execution(&cmdline).await?;

        tracing::trace!("正在执行命令: '{cmdline}'");

        let output = if cfg!(target_os = "windows") {
            std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &format!(
                        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; {}",
                        cmdline
                    ),
                ])
                .output()
        } else {
            std::process::Command::new("sh")
                .args(["-c", &cmdline])
                .output()
        }
        .map_err(CmdError::StdError)?;

        if output.status.success() {
            let out = String::from_utf8_lossy(&output.stdout);
            let err = String::from_utf8_lossy(&output.stderr);

            if out.trim().is_empty() && !err.trim().is_empty() {
                // 标准输出为空但有标准错误
                tracing::warn!("命令成功但产生了 stderr 输出: {}", err.trim());
                Ok(err.trim().to_string())
            } else if out.trim().is_empty() {
                Ok("命令已成功执行，无输出。".to_string())
            } else {
                Ok(out.trim().to_string())
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let msg = if stderr.trim().is_empty() {
                stdout
            } else {
                stderr
            };
            Err(CmdError::StringError(format!(
                "命令 '{}' 执行失败 (状态码 {}): {}",
                cmdline,
                output.status,
                msg.trim(),
            )))
        }
    }
}
