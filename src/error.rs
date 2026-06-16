/// 统一的 Agent 错误类型
///
/// 所有工具（命令执行、文件读写）共享此错误类型。
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}
