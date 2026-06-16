/// 命令行执行工具
///
/// 提供在本地操作系统中安全执行命令的功能，
/// 支持 Windows（PowerShell）和 Linux/macOS（sh）。
/// 在 Windows 上使用 `encoding_rs` 智能处理 GBK/UTF-8 编码转换。
use std::sync::OnceLock;

use encoding_rs::GBK;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::AgentError;

/// 缓存的 PowerShell 版本检测结果：`true` 表示 pwsh 可用
static PW_SH_AVAILABLE: OnceLock<bool> = OnceLock::new();

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

        // 安全检查：阻止明显的危险命令
        check_dangerous_command(&cmdline)?;

        // 权限检查
        crate::permission::confirm_execution(&cmdline)?;

        tracing::trace!("正在执行命令: '{cmdline}'");

        // 跨平台 & 跨版本 Shell 适配
        let output = if cfg!(target_os = "windows") {
            let shell = get_windows_shell();
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

        // 合并 stdout + stderr，确保 LLM 能获取完整的编译警告与进度
        let combined = [stdout.trim(), stderr.trim()]
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");

        if combined.is_empty() {
            Ok("命令已成功执行，无输出。".to_string())
        } else {
            Ok(combined)
        }
    }
}

// ── 安全校验 ──

/// 检查命令是否包含已知的危险模式。
///
/// 这不是完整的安全沙箱，而是对最常见破坏性命令的启发式拦截。
/// 如果命令包含危险模式，直接返回错误，不进入权限确认流程。
fn check_dangerous_command(cmd: &str) -> Result<(), AgentError> {
    let cmd_lower = cmd.to_lowercase();

    // ── 跨平台危险模式 ──
    let cross_platform_dangerous: &[&str] = &[
        // 递归强制删除根目录
        "rm -rf /",
        "rm -rf / --no-preserve-root",
        "rm -r /*",
        // 覆盖磁盘设备
        "dd if=",
        "mkfs.",
        // Fork 炸弹
        ":(){ :|:& };:",
        // curl/wget 管道到 shell（常见攻击向量）
        "curl ",
        "wget ",
    ];

    // curl/wget 管道到 shell 的特殊检测
    if (cmd_lower.contains("curl") || cmd_lower.contains("wget"))
        && (cmd_lower.contains("| sh")
            || cmd_lower.contains("| bash")
            || cmd_lower.contains("| zsh"))
    {
        return Err(AgentError::Other(format!(
            "🚫 安全拦截：检测到远程脚本管道执行 (curl/wget ... | shell)\n   命令: {cmd}"
        )));
    }

    for pattern in cross_platform_dangerous {
        if cmd_lower.contains(pattern) {
            return Err(AgentError::Other(format!(
                "🚫 安全拦截：命令包含危险模式 \"{pattern}\"\n   命令: {cmd}"
            )));
        }
    }

    // ── Windows 特有危险模式 ──
    if cfg!(target_os = "windows") {
        let windows_dangerous: &[&str] = &[
            // 磁盘格式化
            "format ",
            "format c:",
            "format d:",
            // 递归删除系统盘
            "del /f /s c:\\",
            "del /f /s d:\\",
            "rd /s /q c:\\",
            "rd /s /q d:\\",
            // PowerShell 删除根目录
            "remove-item -path c:\\ -recurse",
            "remove-item -path d:\\ -recurse",
            "remove-item c:\\ -recurse",
            "remove-item d:\\ -recurse",
            "ri c:\\ -recurse",
            "ri d:\\ -recurse",
            // 清除事件日志/磁盘
            "clear-disk",
            "format-volume",
            // 禁用安全功能
            "set-executionpolicy unrestricted",
            "disable-computerrestore",
            // 删除注册表关键项
            "remove-item hklm:",
            "remove-item hkcu:",
            "del hklm:",
        ];

        for pattern in windows_dangerous {
            if cmd_lower.contains(pattern) {
                return Err(AgentError::Other(format!(
                    "🚫 安全拦截：命令包含危险模式 \"{pattern}\"\n   命令: {cmd}"
                )));
            }
        }
    }

    // ── Unix 特有危险模式 ──
    if !cfg!(target_os = "windows") {
        let unix_dangerous: &[&str] = &[
            "> /dev/sda",
            "> /dev/hda",
            "> /dev/nvme",
            "mkfs.",
            "chmod 777 /",
            "chown -r root:root /",
            ":(){ :|:& };:",
        ];

        for pattern in unix_dangerous {
            if cmd_lower.contains(pattern) {
                return Err(AgentError::Other(format!(
                    "🚫 安全拦截：命令包含危险模式 \"{pattern}\"\n   命令: {cmd}"
                )));
            }
        }
    }

    Ok(())
}

// ── Shell 选择 ──

/// 获取 Windows 下可用的 Shell，优先 pwsh (PS 7+)，回退 powershell (5.1)。
/// 结果通过 `OnceLock` 缓存，避免每次命令执行都 spawn 检测进程。
fn get_windows_shell() -> &'static str {
    PW_SH_AVAILABLE.get_or_init(|| {
        std::process::Command::new("pwsh")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    });

    if *PW_SH_AVAILABLE.get().unwrap() {
        "pwsh"
    } else {
        "powershell"
    }
}

// ── 编码处理 ──

/// 智能解码命令输出：优先 UTF-8，失败时在 Windows 上回退到 GBK
fn decode_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    // 先尝试 UTF-8
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            // UTF-8 解码失败，在 Windows 上使用 GBK 回退
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
