use exam_server::config::Config;
use exam_server::db::{migrate, open_pool};
use exam_server::error::AppError;
use axum::http::StatusCode;
use sqlx::Row;

#[tokio::test]
async fn opens_exam_db_under_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config::from_data_dir(dir.path());
    cfg.ensure_data_dir().unwrap();
    let pool = open_pool(&cfg.db_path()).await.unwrap();
    migrate(&pool).await.unwrap();
    assert!(cfg.db_path().exists());
    let flags: i64 = sqlx::query("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(flags, 1);
}

#[tokio::test]
async fn foreign_keys_reject_orphan_question() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config::from_data_dir(dir.path());
    cfg.ensure_data_dir().unwrap();
    let pool = open_pool(&cfg.db_path()).await.unwrap();
    migrate(&pool).await.unwrap();
    let err = sqlx::query(
        "INSERT INTO questions (id, bank_id, qtype, stem, answer_json) VALUES ('q1','missing','single','s','{}')",
    )
    .execute(&pool)
    .await
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.to_lowercase().contains("foreign"), "{msg}");
}

#[test]
fn error_body_shape() {
    let err = AppError::new(StatusCode::BAD_REQUEST, "BAD_REQUEST", "班级必填");
    let v = serde_json::to_value(err.body()).unwrap();
    assert_eq!(v["error"]["code"], "BAD_REQUEST");
    assert_eq!(v["error"]["message"], "班级必填");
    assert!(v["error"]["details"].is_object());
}
