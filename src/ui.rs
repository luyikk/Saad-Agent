/// 终端 UI 工具模块
///
/// 封装 console / dialoguer / indicatif，提供统一的美化终端输出。
use console::{style, Alignment, Style, Term};

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
