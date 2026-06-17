/// 应用程序配置
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

/// 默认的 AI 模型名称
pub const DEFAULT_MODEL: &str = "deepseek-v4-flash";

/// 最大对话轮次（默认值）
pub const DEFAULT_MAX_TURNS: usize = 100;

/// 获取最大对话轮次（优先从环境变量 `SAAD_MAX_TURNS` 读取）
pub fn get_max_turns() -> usize {
    std::env::var("SAAD_MAX_TURNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_TURNS)
}

/// 温度参数
pub const DEFAULT_TEMPERATURE: f64 = 0.5;

/// 最大 Token 数
pub const DEFAULT_MAX_TOKENS: usize = 384_000;

/// 对话历史保留的最大消息数（防止 token 超限）
pub const MAX_HISTORY_MESSAGES: usize = 20;

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

/// 努力程度级别（控制 AI 回答的详细程度）
///
/// - `concise`  — 精炼模式，直接给结论，避免啰嗦
/// - `normal`   — 默认，平衡详细度
/// - `elaborate` — 详细模式，展示思考过程和背景
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortLevel {
    Concise,
    Normal,
    Elaborate,
}

impl EffortLevel {
    pub fn preamble_instruction(&self) -> &'static str {
        match self {
            Self::Concise => "- 【重要】用最精炼的方式回答！直接给出方案和代码，避免冗长的背景介绍。用户问什么你就答什么，不要展开无关细节。",
            Self::Normal => "",
            Self::Elaborate => "- 【重要】请提供详细的解释，展示思考过程，讨论替代方案和最佳实践。",
        }
    }

    pub fn max_tokens(&self) -> usize {
        match self {
            Self::Concise => get_max_tokens(),
            Self::Normal => get_max_tokens(),
            Self::Elaborate => get_max_tokens(),
        }
    }

    /// 从字符串解析 EffortLevel
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "concise" | "low" => Some(Self::Concise),
            "normal" | "medium" | "default" => Some(Self::Normal),
            "elaborate" | "high" | "detailed" => Some(Self::Elaborate),
            _ => None,
        }
    }

    /// 显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Concise => "简洁 (concise)",
            Self::Normal => "正常 (normal)",
            Self::Elaborate => "详细 (elaborate)",
        }
    }
}

// ── 动态 effort level ──

/// 0 = 未设置（使用环境变量）, 1 = Concise, 2 = Normal, 3 = Elaborate
static DYNAMIC_EFFORT: AtomicU8 = AtomicU8::new(0);

/// 通过 `/effort` 命令动态设置努力程度（运行时覆盖）
pub fn set_dynamic_effort(level: EffortLevel) {
    let val = match level {
        EffortLevel::Concise => 1,
        EffortLevel::Normal => 2,
        EffortLevel::Elaborate => 3,
    };
    DYNAMIC_EFFORT.store(val, Ordering::SeqCst);
}

/// 获取当前努力程度（动态设置优先，否则回退到环境变量）
pub fn get_effort_level() -> EffortLevel {
    match DYNAMIC_EFFORT.load(Ordering::SeqCst) {
        1 => EffortLevel::Concise,
        2 => EffortLevel::Normal,
        3 => EffortLevel::Elaborate,
        _ => {
            // 0 = 未动态设置，使用环境变量
            match std::env::var("SAAD_EFFORT")
                .unwrap_or_default()
                .to_lowercase()
                .as_str()
            {
                "concise" | "low" => EffortLevel::Concise,
                "elaborate" | "high" | "detailed" => EffortLevel::Elaborate,
                _ => EffortLevel::Normal,
            }
        }
    }
}

/// 权限配置文件路径
pub fn perm_config_path() -> PathBuf {
    PathBuf::from(".saad-agent").join("permission.toml")
}

/// 对话历史文件路径
pub fn history_path() -> PathBuf {
    PathBuf::from(".saad-agent").join("history.json")
}
