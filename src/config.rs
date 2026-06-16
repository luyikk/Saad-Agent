/// 应用程序配置
use std::path::PathBuf;

/// 默认的 AI 模型名称
pub const DEFAULT_MODEL: &str = "deepseek-v4-flash";

/// 最大对话轮次
pub const DEFAULT_MAX_TURNS: usize = 100;

/// 温度参数
pub const DEFAULT_TEMPERATURE: f64 = 0.5;

/// 最大 Token 数
pub const DEFAULT_MAX_TOKENS: usize = 4096;

/// 获取 API Key（优先从环境变量读取）
pub fn get_api_key() -> Result<String, String> {
    match std::env::var("DEEPSEEK_API_KEY") {
        Ok(key) if !key.is_empty() => Ok(key),
        _ => Err("未设置 DEEPSEEK_API_KEY 环境变量！\n请创建 .env 文件或在环境中设置: DEEPSEEK_API_KEY=sk-xxx".to_string()),
    }
}

/// 获取模型名称（优先从环境变量读取）
pub fn get_model_name() -> String {
    std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string())
}

/// 权限配置文件路径
pub fn perm_config_path() -> PathBuf {
    PathBuf::from(".saad-agent").join("permission.toml")
}
