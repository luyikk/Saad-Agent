use std::io::Write;

use anyhow::Result;
use rig::agent::stream_to_stdout;
use rig::completion::Message;
use rig::completion::Usage;
use rig::prelude::*;
use rig::providers::deepseek;
use rig::providers::deepseek::Client as DeepSeekClient;
use rig::streaming::StreamingChat;
use tokio::io::AsyncBufReadExt;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

mod config;
mod permission;
mod tool;

#[tokio::main]
async fn main() -> Result<()> {
    // ---- 初始化日志系统 ----
    let filter_layer = tracing_subscriber::filter::Targets::new()
        .with_default(tracing::Level::INFO) // 默认只显示 INFO 级别，减少噪音
        .with_target("reqwest", LevelFilter::WARN)
        .with_target("hyper_util", LevelFilter::WARN)
        .with_target("h2", LevelFilter::WARN)
        .with_target("rig", LevelFilter::WARN)
        .with_target("saad_agent", LevelFilter::TRACE); // 对本项目启用 TRACE

    let fmt_layer = tracing_subscriber::fmt::layer()
        .pretty()
        .with_span_events(FmtSpan::CLOSE);

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .init();

    // ---- 加载 .env 文件（可选） ----
    let _ = dotenvy::dotenv();

    // ---- 加载持久化的权限状态 ----
    permission::load_permanent_permission();

    // ---- 获取 API Key ----
    let api_key = config::get_api_key().map_err(|e| anyhow::anyhow!(e))?;
    let client: DeepSeekClient = deepseek::Client::new(&api_key)?;

    // ---- 构建 AI Agent ----
    let model_name = config::get_model_name();
    tracing::info!("使用模型: {}", model_name);

    let agent = client
        .agent(&model_name)
        .preamble("你是一个专业的程序员助手，可以执行命令来帮助用户完成任务。")
        .name("Saad")
        .default_max_turns(config::DEFAULT_MAX_TURNS)
        .temperature(config::DEFAULT_TEMPERATURE)
        .max_tokens(config::DEFAULT_MAX_TOKENS as u64)
        .tool(tool::cmd::RunCmd)
        .build();

    // ---- 主交互循环 ----
    let mut history: Vec<Message> = vec![];

    println!("╔══════════════════════════════════════════╗");
    println!("║   Saad Agent - AI 编程助手              ║");
    println!("║   输入你的需求，我会帮你完成！           ║");
    println!("║   输入 /exit 退出                       ║");
    println!("╚══════════════════════════════════════════╝");

    loop {
        print!("\n> ");
        std::io::stdout().flush()?;

        let mut prompt = String::new();
        tokio::io::BufReader::new(tokio::io::stdin())
            .read_line(&mut prompt)
            .await?;

        let prompt = prompt.trim().to_string();

        // 退出命令
        if prompt.eq_ignore_ascii_case("/exit") || prompt.eq_ignore_ascii_case("exit") {
            println!("👋 再见！");
            break;
        }

        // 跳过空输入
        if prompt.is_empty() {
            continue;
        }

        // 发送消息并流式输出回复
        let mut stream = agent.stream_chat(prompt, &history).await;
        let res = stream_to_stdout(&mut stream).await?;

        // 更新对话历史
        if let Some(new_history) = res.history() {
            history.extend_from_slice(new_history);
        }

        // 打印 Token 使用统计
        let usage: Usage = res.usage();
        tracing::debug!("Token 使用情况: {:?}", usage);
    }

    Ok(())
}
