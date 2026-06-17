# Saad Agent

**AI 编程助手** — 基于 DeepSeek 的智能命令行 Agent，使用 [rig](https://crates.io/crates/rig) 框架构建。

[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

---

## 功能特性

- **流式多轮对话** — 实时流式输出，支持推理 (Reasoning) → 工具调用 (ToolCall) → 回答 (Answer) 三阶段展示
- **四级权限系统** — 本次允许 / 会话全部允许 / 永久允许 / 拒绝，危险操作可精细控制，配置持久化到 TOML
- **对话记忆压缩** — 超出消息上限时自动调用 AI 摘要，保持长会话上下文连贯
- **内置工具集**
  - `ExecuteCommand` — 执行系统命令（Windows PowerShell / Unix sh）
  - `ReadFile` — 支持分段、尾部读取
  - `WriteFile` — 覆盖写入文件
  - `EditFile` — 精确字符串替换编辑
  - `GetFileLines` — 快速获取文件行数
- **路径安全防护** — 所有文件操作均包含路径穿越检测，越界访问需额外授权
- **斜杠命令** — `/help`、`/clear`、`/save`、`/load`、`/history`、`/effort`、`/exit`
- **Claude Code 风格 UI** — 阶段分隔线、工具调用缩进展示、Token 统计、转圈动画
- **GBK 兼容** — Windows 下命令输出自动处理 GBK 编码

---

## 安装

### 前置条件

- Rust 工具链 1.85+
- [DeepSeek API Key](https://platform.deepseek.com/)

### 编译

```bash
git clone https://github.com/yourusername/Saad-Agent.git
cd Saad-Agent
cargo build --release
```

二进制文件位于 `target/release/saad-agent.exe` (Windows) 或 `target/release/saad-agent` (Unix)。

### 配置 API Key

```bash
# 设置环境变量
$env:DEEPSEEK_API_KEY="sk-..."        # Windows PowerShell
export DEEPSEEK_API_KEY="sk-..."      # Linux/macOS

# 或在项目根目录创建 .env 文件
echo DEEPSEEK_API_KEY=sk-... > .env
```

---

## 使用

```bash
cargo run
```

启动后进入交互式 REPL，直接输入自然语言描述编程任务：

```
你 → 读取 src/main.rs 并解释它的作用
```

### 斜杠命令

| 命令 | 说明 |
|------|------|
| `/help` | 显示帮助信息 |
| `/clear` | 清空当前对话 |
| `/save` | 手动保存对话历史 |
| `/load` | 加载上次对话历史 |
| `/history` | 查看对话摘要 |
| `/effort concise\|normal\|elaborate` | 切换 Agent 详细程度 |
| `/exit` | 退出程序 |

---

## 配置

通过环境变量覆盖默认值：

| 环境变量 | 默认值 | 说明 |
|----------|--------|------|
| `DEEPSEEK_API_KEY` | — | **必填**，API 密钥 |
| `DEEPSEEK_MODEL` | `deepseek-v4-flash` | 模型名称 |
| `SAAD_MAX_TURNS` | `100` | 每轮最大 Agent 轮次 |
| `SAAD_MAX_TOKENS` | `384000` | 最大 Token 数 |
| `SAAD_MAX_HISTORY` | `20` | 触发记忆压缩的消息数 |
| `SAAD_EFFORT` | `normal` | 详细程度 (`concise` / `normal` / `elaborate`) |

也可在项目根目录创建 `.env` 文件统一管理。

---

## 项目结构

```
src/
├── main.rs              # 入口：tracing 初始化、Agent 构建、REPL 主循环
├── config.rs            # 常量与环境变量读取
├── memory.rs            # ConversationMemory — 消息存储 + AI 摘要压缩
├── permission.rs        # 四级权限系统 (AskOnce / SessionAllow / PermanentAllow / Deny)
├── stream_handler.rs    # 流式响应驱动 → StreamDisplay 状态机
├── command.rs           # 斜杠命令处理
├── error.rs             # 统一错误类型 (AgentError)
├── ui.rs                # 终端渲染：欢迎卡、spinner、StreamDisplay、Token 统计
└── tool/
    ├── mod.rs           # 工具模块入口
    ├── cmd.rs           # ExecuteCommand — 系统命令执行
    └── fs.rs            # ReadFile / WriteFile / EditFile / GetFileLines
```

### 核心数据流

```
用户输入 → REPL 主循环 → build_context() (摘要 + 消息) → agent.stream_chat()
    → stream_handler::process_stream() → StreamDisplay
        (阶段: Reasoning → ToolCall → Answer → Token 统计)
    → memory.extend() → memory.compact() (超限时 AI 摘要)
```

---

## 开发

```bash
cargo check          # 快速编译检查
cargo clippy         # Lint
cargo build          # 完整构建
cargo run            # 构建并运行
```

无测试 (暂无)。

---

## 许可证

本项目采用 [MIT License](LICENSE) 开源。

© 2025 Saad
