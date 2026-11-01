use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum::Json;
use chrono::{Duration, Utc};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db;
use crate::error::AppError;
use crate::AppState;

const SETTING_PASSWORD: &str = "teacher_password_hash";
const SESSION_HOURS: i64 = 24;

#[derive(Deserialize)]
pub struct PasswordBody {
    pub password: String,
}

#[derive(Deserialize)]
pub struct ChangePasswordBody {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub token: String,
    pub expires_at: String,
}

pub async fn setup(
    State(state): State<AppState>,
    Json(body): Json<PasswordBody>,
) -> Result<Json<TokenResponse>, AppError> {
    require_password(&body.password)?;
    if db::get_setting(&state.pool, SETTING_PASSWORD)
        .await
        .map_err(db_err)?
        .is_some()
    {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "ALREADY_SET",
            "教师口令已设置",
        ));
    }
    let hash = hash_password(&body.password)?;
    db::put_setting(&state.pool, SETTING_PASSWORD, &hash)
        .await
        .map_err(db_err)?;
    issue_token(&state).await
}

pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<PasswordBody>,
) -> Result<Json<TokenResponse>, AppError> {
    let stored = db::get_setting(&state.pool, SETTING_PASSWORD)
        .await
        .map_err(db_err)?;
    let Some(stored) = stored else {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "SETUP_REQUIRED",
            "请先设置教师口令",
        ));
    };
    if !verify_password(&body.password, &stored) {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "教师口令错误",
        ));
    }
    issue_token(&state).await
}

pub async fn change_password(
    State(state): State<AppState>,
    Json(body): Json<ChangePasswordBody>,
) -> Result<StatusCode, AppError> {
    require_password(&body.new_password)?;
    let stored = db::get_setting(&state.pool, SETTING_PASSWORD)
        .await
        .map_err(db_err)?
        .ok_or_else(|| {
            AppError::new(StatusCode::CONFLICT, "SETUP_REQUIRED", "请先设置教师口令")
        })?;
    if !verify_password(&body.current_password, &stored) {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "教师口令错误",
        ));
    }
    let hash = hash_password(&body.new_password)?;
    db::put_setting(&state.pool, SETTING_PASSWORD, &hash)
        .await
        .map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn teacher_auth(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let token = teacher_token(&req)?;
    let hash = hash_token(&token);
    let expires = db::teacher_session_expires(&state.pool, &hash)
        .await
        .map_err(db_err)?;
    let Some(expires) = expires else {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "未登录",
        ));
    };
    let exp = chrono::DateTime::parse_from_rfc3339(&expires).map_err(|_| {
        AppError::new(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "教师会话无效")
    })?;
    if exp.with_timezone(&Utc) < Utc::now() {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "教师会话已过期",
        ));
    }
    Ok(next.run(req).await)
}

fn teacher_token(req: &Request<axum::body::Body>) -> Result<String, AppError> {
    if let Some(h) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(t) = h.strip_prefix("Bearer ") {
            if !t.is_empty() {
                return Ok(t.to_string());
            }
        }
    }
    if let Some(c) = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
    {
        for part in c.split(';') {
            let p = part.trim();
            if let Some(t) = p.strip_prefix("exam_teacher=") {
                if !t.is_empty() {
                    return Ok(t.to_string());
                }
            }
        }
    }
    Err(AppError::new(
        StatusCode::UNAUTHORIZED,
        "UNAUTHORIZED",
        "未登录",
    ))
}

async fn issue_token(state: &AppState) -> Result<Json<TokenResponse>, AppError> {
    let token = random_token();
    let expires_at = (Utc::now() + Duration::hours(SESSION_HOURS)).to_rfc3339();
    db::insert_teacher_session(&state.pool, &hash_token(&token), &expires_at)
        .await
        .map_err(db_err)?;
    Ok(Json(TokenResponse { token, expires_at }))
}

fn require_password(password: &str) -> Result<(), AppError> {
    if password.trim().is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "口令不能为空",
        ));
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, AppError> {
    use argon2::password_hash::{PasswordHasher, SaltString};
    use argon2::Argon2;
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "HASH", "口令哈希失败"))
}

fn verify_password(password: &str, stored: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    use argon2::Argon2;
    PasswordHash::new(stored)
        .ok()
        .and_then(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .ok()
        })
        .is_some()
}

fn random_token() -> String {
    let mut buf = [0u8; 32];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

pub fn hash_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    hex::encode(h.finalize())
}

fn db_err(e: sqlx::Error) -> AppError {
    AppError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "DB",
        format!("数据库错误：{e}"),
    )
}
