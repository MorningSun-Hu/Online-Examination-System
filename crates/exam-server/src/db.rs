use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use sqlx::Row;

const SCHEMA: &[&str] = &[
    r#"CREATE TABLE IF NOT EXISTS banks (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    )"#,
    r#"CREATE TABLE IF NOT EXISTS questions (
        id TEXT PRIMARY KEY,
        bank_id TEXT NOT NULL,
        qtype TEXT NOT NULL,
        stem TEXT NOT NULL,
        stem_html TEXT NOT NULL DEFAULT '',
        options_json TEXT NOT NULL DEFAULT '[]',
        answer_json TEXT NOT NULL,
        source TEXT NOT NULL DEFAULT '',
        paper_title TEXT NOT NULL DEFAULT '',
        imported_from TEXT NOT NULL DEFAULT '',
        FOREIGN KEY (bank_id) REFERENCES banks(id)
    )"#,
    r#"CREATE TABLE IF NOT EXISTS students (
        id TEXT PRIMARY KEY,
        student_no TEXT NOT NULL UNIQUE,
        name TEXT NOT NULL,
        class_name TEXT NOT NULL
    )"#,
    r#"CREATE TABLE IF NOT EXISTS exam_tasks (
        id TEXT PRIMARY KEY,
        title TEXT NOT NULL,
        mode TEXT NOT NULL,
        status TEXT NOT NULL,
        duration_minutes INTEGER NOT NULL,
        allow_early_submit INTEGER NOT NULL DEFAULT 0,
        total_score INTEGER NOT NULL,
        pass_score INTEGER NOT NULL,
        published_at TEXT,
        closes_at TEXT
    )"#,
    r#"CREATE TABLE IF NOT EXISTS paper_rules (
        task_id TEXT PRIMARY KEY,
        bank_ids_json TEXT NOT NULL,
        type_counts_json TEXT NOT NULL,
        shuffle_within_section INTEGER NOT NULL DEFAULT 1,
        shuffle_options INTEGER NOT NULL DEFAULT 1,
        type_subtotals_json TEXT,
        score_plan_json TEXT,
        FOREIGN KEY (task_id) REFERENCES exam_tasks(id)
    )"#,
    r#"CREATE TABLE IF NOT EXISTS task_roster (
        task_id TEXT NOT NULL,
        student_id TEXT NOT NULL,
        PRIMARY KEY (task_id, student_id),
        FOREIGN KEY (task_id) REFERENCES exam_tasks(id),
        FOREIGN KEY (student_id) REFERENCES students(id)
    )"#,
    r#"CREATE TABLE IF NOT EXISTS papers (
        id TEXT PRIMARY KEY,
        task_id TEXT NOT NULL,
        student_id TEXT NOT NULL,
        attempt_no INTEGER NOT NULL,
        items_json TEXT NOT NULL,
        created_at TEXT NOT NULL,
        FOREIGN KEY (task_id) REFERENCES exam_tasks(id),
        FOREIGN KEY (student_id) REFERENCES students(id)
    )"#,
    r#"CREATE UNIQUE INDEX IF NOT EXISTS idx_papers_attempt
        ON papers(task_id, student_id, attempt_no)"#,
    r#"CREATE TABLE IF NOT EXISTS attempts (
        id TEXT PRIMARY KEY,
        paper_id TEXT NOT NULL,
        client_id TEXT NOT NULL,
        status TEXT NOT NULL,
        answers_json TEXT NOT NULL DEFAULT '{}',
        objective_score INTEGER,
        total_score INTEGER,
        submitted_at TEXT,
        FOREIGN KEY (paper_id) REFERENCES papers(id)
    )"#,
    r#"CREATE TABLE IF NOT EXISTS sessions (
        token_hash TEXT PRIMARY KEY,
        role TEXT NOT NULL,
        student_id TEXT,
        task_id TEXT,
        client_id TEXT,
        expires_at TEXT NOT NULL
    )"#,
    r#"CREATE TABLE IF NOT EXISTS settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    )"#,
    r#"CREATE TABLE IF NOT EXISTS classes (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL UNIQUE
    )"#,
    r#"CREATE TABLE IF NOT EXISTS exam_papers (
        id TEXT PRIMARY KEY,
        title TEXT NOT NULL,
        bank_ids_json TEXT NOT NULL,
        type_counts_json TEXT NOT NULL,
        shuffle_within_section INTEGER NOT NULL DEFAULT 1,
        shuffle_options INTEGER NOT NULL DEFAULT 1,
        type_subtotals_json TEXT,
        score_plan_json TEXT,
        total_score INTEGER NOT NULL,
        pass_score INTEGER NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    )"#,
];

pub async fn open_pool(db_path: &Path) -> Result<SqlitePool, sqlx::Error> {
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
}

pub async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(pool)
        .await?;
    for stmt in SCHEMA {
        sqlx::query(stmt).execute(pool).await?;
    }
    ensure_column(pool, "exam_tasks", "paper_id", "TEXT").await?;
    ensure_column(pool, "exam_tasks", "parent_id", "TEXT").await?;
    ensure_column(pool, "exam_tasks", "starts_at", "TEXT").await?;
    ensure_column(pool, "exam_papers", "bank_draws_json", "TEXT").await?;
    ensure_column(pool, "paper_rules", "bank_draws_json", "TEXT").await?;
    Ok(())
}

async fn ensure_column(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    decl: &str,
) -> Result<(), sqlx::Error> {
    let sql = format!("PRAGMA table_info({table})");
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    let exists = rows.iter().any(|r| {
        r.try_get::<String, _>("name")
            .ok()
            .map(|n| n == column)
            .unwrap_or(false)
    });
    if !exists {
        let alter = format!("ALTER TABLE {table} ADD COLUMN {column} {decl}");
        sqlx::query(&alter).execute(pool).await?;
    }
    Ok(())
}

pub async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
}

pub async fn put_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO settings(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn insert_teacher_session(
    pool: &SqlitePool,
    token_hash: &str,
    expires_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sessions(token_hash, role, student_id, task_id, client_id, expires_at) VALUES(?, 'teacher', NULL, NULL, NULL, ?)",
    )
    .bind(token_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn teacher_session_expires(
    pool: &SqlitePool,
    token_hash: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>(
        "SELECT expires_at FROM sessions WHERE token_hash = ? AND role = 'teacher'",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

#[derive(Debug, Clone)]
pub struct StudentSessionRow {
    pub student_id: String,
    pub task_id: String,
    pub client_id: String,
    pub expires_at: String,
}

pub async fn insert_student_session(
    pool: &SqlitePool,
    token_hash: &str,
    student_id: &str,
    task_id: &str,
    client_id: &str,
    expires_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sessions(token_hash, role, student_id, task_id, client_id, expires_at) VALUES(?, 'student', ?, ?, ?, ?)",
    )
    .bind(token_hash)
    .bind(student_id)
    .bind(task_id)
    .bind(client_id)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_student_sessions(
    pool: &SqlitePool,
    student_id: &str,
    task_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE role = 'student' AND student_id = ? AND task_id = ?")
        .bind(student_id)
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn student_session(
    pool: &SqlitePool,
    token_hash: &str,
) -> Result<Option<StudentSessionRow>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT student_id, task_id, client_id, expires_at FROM sessions WHERE token_hash = ? AND role = 'student'",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        use sqlx::Row;
        StudentSessionRow {
            student_id: r.get("student_id"),
            task_id: r.get("task_id"),
            client_id: r.get("client_id"),
            expires_at: r.get("expires_at"),
        }
    }))
}

pub async fn expire_student_sessions_for_task(
    pool: &SqlitePool,
    task_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE sessions SET expires_at = '1970-01-01T00:00:00Z' WHERE role = 'student' AND task_id = ?",
    )
    .bind(task_id)
    .execute(pool)
    .await?;
    Ok(())
}
