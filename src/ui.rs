use std::borrow::Cow;
/// 终端 UI 工具模块
///
/// 封装 console / dialoguer / indicatif，提供统一的美化终端输出。
use std::io::{self, Write};

use console::{style, Alignment, Style, Term};
use rig::completion::Usage;

// ── 快捷样式 ──

#[inline]
pub fn s_success(s: &str) -> String {
    style(s).green().to_string()
}

#[inline]
pub fn s_warn(s: &str) -> String {
    style(s).yellow().to_string()
}

#[inline]
pub fn s_error(s: &str) -> String {
    style(s).red().to_string()
}

#[inline]
pub fn s_dim(s: &str) -> String {
    style(s).dim().to_string()
}

#[inline]
pub fn s_cmd(s: &str) -> String {
    style(s).cyan().bold().to_string()
}

// ── 格式化 ──

/// 将 token 数量格式化为人类可读的 k/m 单位
///
/// - >= 1_000_000 → "1.23m"
/// - >= 1_000     → "1.23k"
/// - 否则保留原值
#[allow(clippy::cast_precision_loss)]
fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        let val = n as f64 / 1_000_000.0;
        if (val * 100.0).round() % 100.0 == 0.0 {
            format!("{:.0}m", val)
        } else {
            format!("{:.2}m", val)
        }
    } else if n >= 1_000 {
        let val = n as f64 / 1_000.0;
        if (val * 100.0).round() % 100.0 == 0.0 {
            format!("{:.0}k", val)
        } else {
            format!("{:.2}k", val)
        }
    } else {
        n.to_string()
    }
}

// ── 布局 ──

/// 获取终端宽度
pub fn term_width() -> usize {
    Term::stdout().size().1 as usize
}

/// 打印水平分隔线
pub fn print_divider() {
    let w = term_width().min(80);
    println!("{}", s_dim(&"\u{2500}".repeat(w)));
}

/// 打印空行
pub fn print_spacer() {
    println!();
}

// ── 欢迎 / 帮助 / 退出 ──

/// 打印现代化欢迎卡片
///
/// 使用 `console::pad_str` 而非标准库 `format!("{:^w$}")`，
/// 以正确处理中文/emoji 等双宽字符的居中排版。
pub fn print_welcome(history_count: usize) {
    let term = Term::stdout();
    let width = term.size().1 as usize;

    // 卡片宽度：最小 46 以容纳中文内容，最大 74 保持精致
    let card_w = width.clamp(46, 74);
    let inner = card_w.saturating_sub(2); // 扣除 │ 边框

    // CJK / emoji 宽度感知的居中辅助闭包
    let center =
        |s: &str| -> String { console::pad_str(s, inner, Alignment::Center, None).to_string() };

    // 预制 Style
    let accent = Style::new().cyan();
    let bold = Style::new().cyan().bold();
    let dim = Style::new().dim();

    // 带边框的行
    let row = |content: &str, s: &Style| -> String {
        format!(
            "{}{}{}",
            s.apply_to("│"),
            s.apply_to(content),
            s.apply_to("│")
        )
    };

    let top_border = format!("╭{}╮", "─".repeat(inner));
    let bot_border = format!("╰{}╯", "─".repeat(inner));
    let spacer_row = format!("│{}│", " ".repeat(inner));

    println!();
    println!("{}", accent.apply_to(&top_border));

    // 标题区
    println!("{}", accent.apply_to(&spacer_row));
    println!("{}", row(&center("🚀  Saad Agent"), &bold));
    println!("{}", row(&center("AI 编程助手 · 智能伙伴"), &accent));

    // 描述区
    println!("{}", accent.apply_to(&spacer_row));
    println!("{}", row(&center("输入你的需求，我来帮你完成！"), &accent));

    // 快捷键区
    println!("{}", accent.apply_to(&spacer_row));
    println!("{}", row(&center("/help 命令  ·  /exit 退出"), &dim));

    // 闭合
    println!("{}", accent.apply_to(&spacer_row));
    println!("{}", accent.apply_to(&bot_border));

    // 历史提示
    if history_count > 0 {
        println!(
            "  {} 已加载 {} 条历史消息 — 继续对话",
            s_dim("📂"),
            style(history_count).yellow().bold()
        );
    }
    println!();
}

pub fn print_help() {
    print_spacer();
    println!("{}", style("◆  命令列表").bold().underlined());
    print_spacer();

    let commands: &[(&str, &str)] = &[
        ("/help", "显示此帮助信息"),
        (
            "/effort <level>",
            "设置回答详细程度 (concise/normal/elaborate)",
        ),
        ("/clear", "清空对话历史"),
        ("/save", "保存对话历史到磁盘"),
        ("/load", "从磁盘加载对话历史"),
        ("/history", "显示当前对话历史统计"),
        ("/exit", "退出程序"),
    ];

    let max_cmd = commands.iter().map(|c| c.0.len()).max().unwrap_or(0);

    for (cmd, desc) in commands {
        println!(
            "  {} {} {}",
            s_cmd(cmd),
            " ".repeat(max_cmd - cmd.len() + 2),
            s_dim(desc)
        );
    }

    print_spacer();
    println!("  {}  Ctrl+C 优雅退出（自动保存历史）", s_dim("💡"));
    print_spacer();
}

pub fn print_goodbye(saved: bool) {
    print_spacer();
    if saved {
        println!("{} 对话历史已自动保存", s_dim("💾"));
    }
    println!("{} 再见！", style("👋").cyan());
    print_spacer();
}

// ── 状态消息 ──

pub fn print_info(msg: &str) {
    println!("{} {}", s_dim("℗"), msg);
}

pub fn print_success(msg: &str) {
    println!("{} {}", s_success("✅"), msg);
}

pub fn print_warning(msg: &str) {
    println!("{} {}", s_warn("⚠️"), msg);
}

pub fn print_error(msg: &str) {
    println!("{} {}", s_error("❌"), msg);
}

// ── 权限弹窗（基于 dialoguer::Select） ──

pub fn select_permission(action_desc: &str, detail: &str) -> Option<usize> {
    print_spacer();

    println!(
        "{} {}",
        style("⚠").yellow().bold(),
        style(action_desc).yellow()
    );
    println!("  {} {}", s_dim("🔧"), s_dim(detail));
    print_spacer();

    let items = &[
        "允许本次执行",
        "本次会话全部允许",
        "永久允许（不再询问）",
        "拒绝",
    ];

    let selection = dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt("请选择操作")
        .items(items)
        .default(0)
        .interact()
        .ok();

    print_spacer();
    selection
}

// ── Spinner（基于 indicatif） ──

pub fn new_spinner(msg: &str) -> indicatif::ProgressBar {
    let pb = indicatif::ProgressBar::new_spinner();
    pb.set_style(
        indicatif::ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}

// ── 流式响应渲染器 ──

/// AI 流式响应的终端渲染器。
///
/// 对标 Claude Code CLI 的阶段展示效果，支持：
/// - 🧠 深度思考（Thinking/Reasoning 流式输出）
/// - 🔧 工具调用（展示工具名 + 参数摘要）
/// - 📥 工具结果（展示返回内容预览）
/// - 💬 回答（最终文本流式输出）
/// - 📊 Token 统计
pub struct StreamDisplay {
    term: Term,
    state: StreamPhase,
    /// 分隔线宽度（取终端宽度与 80 的较小值）
    line_w: usize,
    /// 工具调用计数器（用于编号）
    tool_call_count: u32,
    /// 最大对话轮次（用于显示进度如 #1/100）
    max_turns: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamPhase {
    /// 尚未输出任何内容
    Idle,
    /// 正在输出推理链（DeepSeek-R1 等思维链模型）
    Reasoning,
    /// 正在输出工具调用参数（ToolCallDelta 流）
    ToolCallDelta,
    /// 工具调用已确认
    ToolCall,
    /// 正在输出最终回答
    Answer,
}

impl StreamDisplay {
    /// 创建新的流式渲染器
    pub fn new(max_turns: usize) -> Self {
        let width = Term::stdout().size().1 as usize;
        Self {
            term: Term::stdout(),
            state: StreamPhase::Idle,
            line_w: width.min(80),
            tool_call_count: 0,
            max_turns,
        }
    }

    // ── 内部：阶段切换 ──

    /// 结束当前行（如果正在行内输出文本），保证光标在新行开头
    fn end_line(&mut self) -> io::Result<()> {
        match self.state {
            StreamPhase::Answer | StreamPhase::Reasoning | StreamPhase::ToolCallDelta => {
                // 之前正在流式输出文本，光标在行内，需要换行收尾
                writeln!(self.term)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// 进入新阶段：收尾上一阶段 + 打印阶段头部
    fn enter_phase(&mut self, icon: &str, label: &str, target: StreamPhase) -> io::Result<()> {
        if self.state == target {
            return Ok(());
        }
        self.end_line()?;
        // 只在从 Idle 进入时才不加空行，否则用一个空行分隔
        if self.state != StreamPhase::Idle {
            writeln!(self.term)?;
        }
        self.print_phase_header(icon, label)?;
        self.state = target;
        Ok(())
    }

    /// 打印阶段头部：`🧠 深度思考 ──────────────────────`
    fn print_phase_header(&mut self, icon: &str, label: &str) -> io::Result<()> {
        let icon_w = console::measure_text_width(icon);
        let label_w = console::measure_text_width(label);
        // 布局: icon + 空格 + label + 空格 + 填充线
        let used = icon_w + 1 + label_w + 1;
        let fill = self.line_w.saturating_sub(used);
        writeln!(
            self.term,
            "{} {} {}",
            icon,
            style(label).bold(),
            s_dim(&"─".repeat(fill))
        )
    }

    /// 打印工具调用信息行
    fn print_tool_call(&mut self, name: &str, args_preview: &str) -> io::Result<()> {
        self.end_line()?;
        self.tool_call_count += 1;
        writeln!(
            self.term,
            "{} {} {}",
            style("⏺").cyan(),
            style(&format!("#{}", self.tool_call_count)).cyan().dim(),
            style(name).cyan().bold()
        )?;
        if !args_preview.is_empty() {
            let avail = self.line_w.saturating_sub(4); // "  " + "└ " = 4 列
            writeln!(
                self.term,
                "  {} {}",
                s_dim("└"),
                s_dim(&truncate_str(args_preview, avail))
            )?;
        }
        self.state = StreamPhase::ToolCall;
        Ok(())
    }

    /// 打印工具返回结果（支持多行输出）
    ///
    /// 对多行内容：`✓` 单独一行，后续内容逐行带缩进渲染，
    /// 最多显示 10 行，超出部分显示截断提示。
    fn print_tool_result(&mut self, success: bool, summary: &str) -> io::Result<()> {
        let icon = if success {
            s_success("✓ success")
        } else {
            s_error("✗ error")
        };
        writeln!(self.term, "    {icon}")?;

        // 多行内容逐行渲染，统一缩进
        const MAX_LINES: usize = 10;
        let lines: Vec<&str> = summary.lines().collect();
        let total = lines.len();
        let display_lines = if total > MAX_LINES {
            &lines[..MAX_LINES]
        } else {
            &lines
        };

        for line in display_lines {
            let trimmed = truncate_str(line, self.line_w.saturating_sub(6));
            writeln!(self.term, "    {}", s_dim(&trimmed))?;
        }

        if total > MAX_LINES {
            writeln!(
                self.term,
                "    {}",
                s_dim(&format!("... (还有 {} 行已省略)", total - MAX_LINES))
            )?;
        } else if total > 1 {
            writeln!(self.term, "    {}", s_dim(&format!("({total} 行)",)))?;
        }

        Ok(())
    }

    // ── 公开方法，供 main.rs 的 stream 循环调用 ──

    /// 处理推理链完整块
    pub fn on_reasoning(&mut self, text: &str) -> io::Result<()> {
        self.enter_phase("🧠", "深度思考", StreamPhase::Reasoning)?;
        write!(self.term, "{text}")?;
        self.term.flush()
    }

    /// 处理推理链增量（流式 token）
    pub fn on_reasoning_delta(&mut self, text: &str) -> io::Result<()> {
        self.enter_phase("🧠", "深度思考", StreamPhase::Reasoning)?;
        write!(self.term, "{text}")?;
        self.term.flush()
    }

    /// 处理回答 token
    pub fn on_answer(&mut self, text: &str) -> io::Result<()> {
        self.enter_phase("💬", "回答", StreamPhase::Answer)?;
        write!(self.term, "{text}")?;
        self.term.flush()
    }

    /// 处理工具调用：显示工具名 + 参数摘要
    pub fn on_tool_call(&mut self, name: &str, args_preview: &str) -> io::Result<()> {
        self.print_tool_call(name, args_preview)
    }

    /// 处理工具调用的增量参数流
    pub fn on_tool_call_delta(&mut self, delta: &str) -> io::Result<()> {
        if self.state != StreamPhase::ToolCallDelta {
            self.end_line()?;
            self.state = StreamPhase::ToolCallDelta;
        }
        write!(self.term, "{}", s_dim(delta))?;
        self.term.flush()
    }

    /// 处理工具执行结果
    pub fn on_tool_result(&mut self, success: bool, summary: &str) -> io::Result<()> {
        self.print_tool_result(success, summary)
    }

    /// 处理 CompletionCall：打印本轮 token 用量，含进度 #n/max
    pub fn on_completion_call(&mut self, call_index: u32, usage: Option<Usage>) {
        // 格式: `  ⏺  Turn #1/100 · 1.2k input · 0.3k output`
        if let Some(u) = usage {
            let turn = call_index + 1; // call_index 是 0-based，显示用 1-based
            let _ = writeln!(self.term);
            let _ = writeln!(
                self.term,
                "  {}  {} {}  {} {}  {} {}",
                s_dim("⏺"),
                s_dim("Turn"),
                s_dim(&format!("#{}/{}", turn, self.max_turns)),
                s_dim("·"),
                s_dim(&format!("{} input", fmt_tokens(u.input_tokens))),
                s_dim("·"),
                s_dim(&format!("{} output", fmt_tokens(u.output_tokens))),
            );
        }
    }

    /// 处理流错误
    pub fn on_error(&mut self, err: &str) {
        let _ = writeln!(self.term);
        let _ = writeln!(self.term, "   {} {}", s_error("✗"), s_error(err));
    }

    /// 打印最终统计信息并收尾
    pub fn finalize(&mut self, usage: &Usage) {
        let _ = writeln!(self.term);
        if usage.total_tokens > 0 {
            let dash = "─".repeat(self.line_w.saturating_sub(30).max(10));
            let _ = writeln!(
                self.term,
                "{}  {}",
                s_dim("📊"),
                s_dim(&format!(
                    "总计 —{} 输入 {} · 输出 {} · {} tokens",
                    dash,
                    fmt_tokens(usage.input_tokens),
                    fmt_tokens(usage.output_tokens),
                    fmt_tokens(usage.total_tokens)
                ))
            );
        }
        let _ = writeln!(self.term);
    }
}

// ── 辅助函数 ──

/// 辅助函数：安全截断字符串，使显示宽度不超过 max_width。
///
/// 使用 `console::measure_text_width` 而非 `unicode_width` 直接计算，
/// 保证与 console crate 的宽度计算一致，避免中文被错误截断。
fn truncate_str(s: &str, max_width: usize) -> Cow<'_, str> {
    if max_width == 0 {
        return Cow::Borrowed("");
    }

    let total_w = console::measure_text_width(s);
    if total_w <= max_width {
        return Cow::Borrowed(s);
    }

    // 二分查找合适的截断位置
    let mut lo = 0;
    let mut hi = s.len();

    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        // 确保 mid 落在字符边界
        let safe_mid = find_char_boundary(s, mid);

        let prefix = &s[..safe_mid];
        let w = console::measure_text_width(prefix);

        if w <= max_width {
            lo = safe_mid;
        } else {
            hi = find_char_boundary(s, safe_mid.saturating_sub(1));
        }
    }

    // lo 是满足宽度 ≤ max_width 的最长前缀的字节位置
    // 截取后添加 "…" 提示
    let suffix = "…";
    if lo == 0 {
        Cow::Owned(suffix.to_string())
    } else {
        // 为后缀腾出空间
        let suffix_w = console::measure_text_width(suffix);
        let avail = max_width.saturating_sub(suffix_w);
        let new_len = if avail == 0 {
            lo
        } else {
            std::cmp::min(lo, avail)
        };
        let new_len = find_char_boundary(s, new_len);
        if new_len == 0 {
            Cow::Owned(suffix.to_string())
        } else {
            Cow::Owned(format!("{}{}", &s[..new_len], suffix))
        }
    }
}

/// 找到离 `pos` 最近的合法 UTF-8 字符边界（向左查找）
fn find_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos.min(s.len());
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}
