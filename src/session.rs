//! Session 管理模块
//!
//! 基于 SQLite (sqlx) 的对话 session 存储，支持多 session 管理。
//! 所有 I/O 函数均为 async，返回 `anyhow::Result`。

use anyhow::{Context, Result};
use rig::message::Message;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::Row;

use crate::config;

// ============================================================
// SessionMeta — 列表展示用的轻量元数据
// ============================================================

/// Session 元数据（不含 messages 本体，仅用于列表展示）
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SessionMeta {
    pub id: String,
    pub created_at: String,
    pub last_updated: String,
    pub msg_count: usize,
    pub title: String,
}

// ============================================================
// 连接管理
// ============================================================

/// 打开（或创建）SQLite 数据库并确保表结构存在
pub async fn open_db() -> Result<SqlitePool> {
    let db_path = config::db_path();

    // 确保父目录存在
    if let Some(parent) = db_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("无法创建 .saad-agent 目录")?;
    }

    let options = SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .with_context(|| format!("无法连接到数据库: {}", db_path.display()))?;

    sqlx::query(
        r"CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL,
            last_updated TEXT NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            summary TEXT NOT NULL DEFAULT '',
            messages_json TEXT NOT NULL DEFAULT '[]',
            msg_count INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(&pool)
    .await
    .context("无法创建 sessions 表")?;

    tracing::debug!("数据库已就绪: {}", db_path.display());
    Ok(pool)
}

// ============================================================
// Session ID 生成
// ============================================================

/// 生成唯一的 session ID：`<YYYYMMDD>-<HHMMSS>-<8hex>`
pub fn generate_id() -> String {
    let now = chrono::Local::now();
    let date = now.format("%Y%m%d");
    let time = now.format("%H%M%S");
    // 使用简单的随机 hex（避免引入额外依赖）
    let random: u32 = {
        let buf = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        (buf.wrapping_mul(1_103_515_245).wrapping_add(12_345))
            ^ (std::process::id().wrapping_mul(2_654_435_761))
    };
    format!("{date}-{time}-{random:08x}")
}

// ============================================================
// CRUD
// ============================================================

/// 保存 session（INSERT OR REPLACE）
pub async fn save(
    pool: &SqlitePool,
    id: &str,
    title: &str,
    summary: Option<&str>,
    messages_json: &str,
    msg_count: usize,
) -> Result<()> {
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let summary = summary.unwrap_or("");

    // 先检查是否已存在（用于决定保留哪个 created_at）
    let existing: Option<String> =
        sqlx::query_scalar("SELECT created_at FROM sessions WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .context("查询已有 session 失败")?;

    let created_at = existing.unwrap_or_else(|| now.clone());

    sqlx::query(
        r"INSERT OR REPLACE INTO sessions (id, created_at, last_updated, title, summary, messages_json, msg_count)
          VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&created_at)
    .bind(&now)
    .bind(title)
    .bind(summary)
    .bind(messages_json)
    .bind(msg_count as i64)
    .execute(pool)
    .await
    .with_context(|| format!("保存 session {id} 失败"))?;

    tracing::debug!("session {id} 已保存 ({msg_count} 条消息)");
    Ok(())
}

/// 加载 session，返回 (messages, summary, title)
pub async fn load(pool: &SqlitePool, id: &str) -> Result<(Vec<Message>, Option<String>, String)> {
    let row = sqlx::query("SELECT messages_json, summary, title FROM sessions WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("查询 session {id} 失败"))?
        .with_context(|| format!("session {id} 不存在"))?;

    let messages_json: String = row.get("messages_json");
    let summary: String = row.get("summary");
    let title: String = row.get("title");

    let messages: Vec<Message> =
        serde_json::from_str(&messages_json).context("反序列化 messages 失败")?;

    let summary = if summary.is_empty() {
        None
    } else {
        Some(summary)
    };

    tracing::debug!("session {id} 已加载 ({} 条消息)", messages.len());
    Ok((messages, summary, title))
}

/// 列出所有 session（按 last_updated DESC），不含 messages 本体
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<SessionMeta>> {
    let rows = sqlx::query(
        r"SELECT id, created_at, last_updated, title, msg_count
          FROM sessions
          ORDER BY last_updated DESC",
    )
    .fetch_all(pool)
    .await
    .context("查询 session 列表失败")?;

    let mut sessions = Vec::with_capacity(rows.len());
    for row in rows {
        let count: i64 = row.get("msg_count");
        sessions.push(SessionMeta {
            id: row.get("id"),
            created_at: row.get("created_at"),
            last_updated: row.get("last_updated"),
            title: row.get("title"),
            msg_count: count as usize,
        });
    }

    Ok(sessions)
}

/// 获取最近的 session
pub async fn most_recent(pool: &SqlitePool) -> Result<Option<SessionMeta>> {
    let sessions = list_all(pool).await?;
    Ok(sessions.into_iter().next())
}

/// 删除指定 session
pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .with_context(|| format!("删除 session {id} 失败"))?;

    if result.rows_affected() == 0 {
        tracing::warn!("session {id} 不存在，无需删除");
    } else {
        tracing::debug!("session {id} 已删除");
    }
    Ok(())
}
