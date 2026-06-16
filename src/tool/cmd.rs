/// 命令行执行工具
///
/// 提供在本地操作系统中安全执行命令的功能，
/// 支持 Windows（PowerShell）和 Linux/macOS（sh）。
/// 在 Windows 上使用 `encoding_rs` 智能处理 GBK/UTF-8 编码转换。
use std::sync::OnceLock;

use encoding_rs::GBK;
use regex::Regex;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;

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

        // 安全检查：检测危险模式，弹框让用户确认（四级权限选择）
        confirm_dangerous_command(&cmdline)?;

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
/// 如果匹配到危险模式，通过 `permission::confirm_dangerous` 弹出四级权限选择对话框：
/// 1. 允许本次执行
/// 2. 本次会话全部允许
/// 3. 永久允许（不再询问）
/// 4. 拒绝
///
/// 用户拒绝时返回错误。
fn confirm_dangerous_command(cmd: &str) -> Result<(), AgentError> {
    let reason = match detect_dangerous_pattern(cmd) {
        Some(r) => r,
        None => return Ok(()),
    };

    crate::permission::confirm_dangerous(&reason, cmd)
}

/// 检测命令是否匹配危险模式，返回危险原因描述
fn detect_dangerous_pattern(cmd: &str) -> Option<String> {
    let cmd_trimmed = cmd.trim();
    if cmd_trimmed.is_empty() {
        return None;
    }

    // 1. 拦截真正危险的 Shell 注入与命令替换（放行合法的 ; 和 |）
    let injection_patterns = [
        r"`",     // 拦截反引号命令替换 `...`
        r"\$\(",  // 拦截命令替换 $(...)
        r">\s*/", // 拦截重定向到绝对路径 (如 > /etc/passwd)
    ];
    for pattern in &injection_patterns {
        if Regex::new(pattern).unwrap().is_match(cmd_trimmed) {
            return Some(format!(
                "检测到潜在的命令注入或危险重定向 — 匹配模式: \"{}\"",
                pattern
            ));
        }
    }

    // 2. 跨平台高危模式（使用正则匹配参数顺序和空格）
    let cross_platform_patterns: &[(&str, &str)] = &[
        (r"\brm\s+.*-r.*-f.*\s+/\b", "递归强制删除根目录 (rm -rf /)"),
        (
            r"\brm\s+.*--no-preserve-root\b",
            "递归删除根目录（绕过保护）",
        ),
        (r">\s*/dev/sd[a-z]", "覆写磁盘设备"),
        (r"\bmkfs\.", "格式化文件系统"),
        (r":\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:", "Fork 炸弹"),
    ];
    for (pattern, desc) in cross_platform_patterns {
        if Regex::new(pattern).unwrap().is_match(cmd_trimmed) {
            return Some(format!("{} — 匹配模式: \"{}\"", desc, pattern));
        }
    }

    // 3. 构建全栈开发安全白名单
    let allowed_commands: HashSet<&str> = [
        // 基础文件与目录操作
        "ls",
        "dir",
        "pwd",
        "cd",
        "tree",
        "cat",
        "head",
        "tail",
        "touch",
        "mkdir",
        "cp",
        "mv",
        "rm",
        // 文本搜索与处理
        "grep",
        "find",
        "awk",
        "sed",
        "wc",
        "sort",
        "uniq",
        "diff",
        // 系统状态与网络调试
        "ps",
        "top",
        "htop",
        "df",
        "du",
        "free",
        "uname",
        "whoami",
        "hostname",
        "date",
        "uptime",
        "env",
        "echo",
        "ping",
        "curl",
        "wget",
        "netstat",
        "ss",
        "ip",
        "ifconfig",
        "nslookup",
        "dig",
        "traceroute",
        "telnet",
        // Rust 生态
        "cargo",
        "rustc",
        "rustup",
        "clippy",
        "rustfmt",
        // Node.js 生态
        "node",
        "npm",
        "yarn",
        "pnpm",
        "npx",
        "bun",
        "tsc",
        // Python 生态
        "python",
        "python3",
        "pip",
        "pip3",
        "poetry",
        "conda",
        "mypy",
        "pytest",
        // 版本控制与工具链
        "git",
        "git-lfs",
        "tar",
        "unzip",
        "zip",
        "jq",
        "ffmpeg",
        "make",
        // 容器化与云服务
        "docker",
        "docker-compose",
        "kubectl",
        "helm",
        // Windows / PowerShell 专属
        "Select-Object",
        "Where-Object",
        "ForEach-Object",
        "Out-File",
        "ConvertTo-Json",
        "ConvertFrom-Json",
        "Get-Content",
        "Get-ChildItem",
        "Set-Location",
    ]
    .iter()
    .cloned()
    .collect();

    // 4. 按 ; 或 | 拆分命令，确保每个子命令都在白名单内
    let split_pattern = Regex::new(r"\s*[;|]\s*").unwrap();
    let sub_commands = split_pattern.split(cmd_trimmed);

    for sub_cmd in sub_commands {
        let sub_cmd = sub_cmd.trim();
        if sub_cmd.is_empty() {
            continue;
        }

        // 提取基础命令名称，兼容绝对路径（如 /usr/bin/rm）
        let base_cmd = sub_cmd.split_whitespace().next().unwrap_or("");
        let cmd_name = base_cmd.split('/').next_back().unwrap_or("");

        if !cmd_name.is_empty() && !allowed_commands.contains(cmd_name) {
            return Some(format!("子命令 \"{}\" 不在允许执行的白名单中", cmd_name));
        }
    }

    // 5. 平台特有检测（示例：Unix 权限修改）
    #[cfg(not(target_os = "windows"))]
    {
        let unix_patterns: &[(&str, &str)] = &[
            (r"\bchmod\s+777\s+/\b", "修改根目录权限为 777"),
            (
                r"\bchown\s+-R\s+root:root\s+/\b",
                "递归修改根目录所有者为 root",
            ),
        ];
        for (pattern, desc) in unix_patterns {
            if Regex::new(pattern).unwrap().is_match(cmd_trimmed) {
                return Some(format!("{} — 匹配模式: \"{}\"", desc, pattern));
            }
        }
    }

    None
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
