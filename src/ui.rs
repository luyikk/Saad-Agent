/// 终端 UI 工具模块
///
/// 封装 console / dialoguer / indicatif，提供统一的美化终端输出。
use console::{style, Term};
use dialoguer::{theme::ColorfulTheme, Select};
use indicatif::{ProgressBar, ProgressStyle};

// ── 颜色常量 ──

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

pub fn print_welcome(history_count: usize) {
    let term = Term::stdout();
    let width = term.size().1 as usize;
    let w = width.clamp(30, 60);

    let border_top = format!("\u{256d}{:\u{2500}^w$}\u{256e}", "", w = w - 2);
    let border_bot = format!("\u{2570}{:\u{2500}^w$}\u{256f}", "", w = w - 2);

    println!();
    println!("{}", style(border_top).cyan());
    println!(
        "{}",
        style(format!(
            "\u{2502}{:^w$}\u{2502}",
            "Saad Agent — AI 编程助手",
            w = w - 2
        ))
        .cyan()
        .bold()
    );
    println!(
        "{}",
        style(format!(
            "\u{2502}{:^w$}\u{2502}",
            "输入你的需求，我来帮你完成！",
            w = w - 2
        ))
        .cyan()
    );
    println!(
        "{}",
        style(format!(
            "\u{2502}{:^w$}\u{2502}",
            "/help 查看命令",
            w = w - 2
        ))
        .dim()
    );
    println!("{}", style(border_bot).cyan());

    if history_count > 0 {
        println!(
            "  {} 已加载 {} 条历史消息",
            s_dim("\u{1f4c2}"),
            style(history_count).yellow()
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
    println!(
        "  {}  Ctrl+C 优雅退出（自动保存历史）",
        s_dim("\u{1f4a1}")
    );
    print_spacer();
}

pub fn print_goodbye(saved: bool) {
    print_spacer();
    if saved {
        println!("{} 对话历史已自动保存", s_dim("\u{1f4be}"));
    }
    println!("{} 再见！", style("\u{1f44b}").cyan());
    print_spacer();
}

// ── 状态消息 ──

pub fn print_info(msg: &str) {
    println!("{} {}", s_dim("\u{2117}"), msg);
}

pub fn print_success(msg: &str) {
    println!("{} {}", s_success("\u{2705}"), msg);
}

pub fn print_warning(msg: &str) {
    println!("{} {}", s_warn("\u{26a0}\u{fe0f}"), msg);
}

pub fn print_error(msg: &str) {
    println!("{} {}", s_error("\u{274c}"), msg);
}

// ── 权限弹窗（基于 dialoguer::Select） ──

pub fn select_permission(action_desc: &str, detail: &str) -> Option<usize> {
    print_spacer();

    // 打印警告描述
    println!(
        "{} {}",
        style("\u{26a0}").yellow().bold(),
        style(action_desc).yellow()
    );
    println!("  {} {}", s_dim("\u{1f527}"), s_dim(detail));
    print_spacer();

    let items = &[
        "允许本次执行",
        "本次会话全部允许",
        "永久允许（不再询问）",
        "拒绝",
    ];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("请选择操作")
        .items(items)
        .default(0)
        .interact()
        .ok();

    print_spacer();
    selection
}

// ── Spinner（基于 indicatif） ──

pub fn new_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&[
                "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}",
                "\u{2834}", "\u{2826}", "\u{2827}", "\u{2807}", "\u{280f}",
            ]),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}

