/// 应用程序配置
use std::path::PathBuf;

/// 默认的 AI 模型名称
pub const DEFAULT_MODEL: &str = "deepseek-v4-flash";

/// 最大对话轮次
pub const DEFAULT_MAX_TURNS: usize = 100;

/// 温度参数
pub const DEFAULT_TEMPERATURE: f64 = 0.5;

/// 最大 Token 数
pub const DEFAULT_MAX_TOKENS: usize = 384_000;

/// 对话历史保留的最大消息数（防止 token 超限）
pub const MAX_HISTORY_MESSAGES: usize = 40;

/// 获取 API Key（优先从环境变量读取）
pub fn get_api_key() -> Result<String, String> {
    match std::env::var("DEEPSEEK_API_KEY") {
        Ok(key) if !key.is_empty() => Ok(key),
        _ => Err(
            "未设置 DEEPSEEK_API_KEY 环境变量！\n请创建 .env 文件或在环境中设置: DEEPSEEK_API_KEY=sk-xxx"
                .to_string(),
        ),
    }
}

/// 获取模型名称（优先从环境变量读取）
pub fn get_model_name() -> String {
    std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string())
}

/// 获取最大 Token 数（优先从环境变量 `SAAD_MAX_TOKENS` 读取）
pub fn get_max_tokens() -> usize {
    std::env::var("SAAD_MAX_TOKENS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_TOKENS)
}

/// 获取最大历史消息数（优先从环境变量 `SAAD_MAX_HISTORY` 读取）
pub fn get_max_history_messages() -> usize {
    std::env::var("SAAD_MAX_HISTORY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MAX_HISTORY_MESSAGES)
}

/// 权限配置文件路径
pub fn perm_config_path() -> PathBuf {
    PathBuf::from(".saad-agent").join("permission.toml")
}

/// 对话历史文件路径
pub fn history_path() -> PathBuf {
    PathBuf::from(".saad-agent").join("history.json")
}
