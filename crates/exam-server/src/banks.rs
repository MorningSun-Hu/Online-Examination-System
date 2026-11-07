use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::Json;
use bank_html::{parse_html, pretty_print, Answer, OptionItem, QType, Question};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use ulid::Ulid;

use crate::error::AppError;
use crate::AppState;

#[derive(Deserialize)]
pub struct CreateBankBody {
    pub name: String,
}

#[derive(Serialize)]
pub struct BankSummary {
    pub id: String,
    pub name: String,
    pub question_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
pub struct BankList {
    pub banks: Vec<BankSummary>,
}

#[derive(Serialize)]
pub struct BankDetail {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub questions: Vec<StoredQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredQuestion {
    pub id: String,
    pub bank_id: String,
    pub qtype: QType,
    pub stem: String,
    pub stem_html: String,
    pub options: Vec<OptionItem>,
    pub answer: Answer,
    pub source: String,
    pub paper_title: String,
    pub imported_from: String,
}

#[derive(Deserialize)]
pub struct QuestionBody {
    pub qtype: QType,
    pub stem: String,
    #[serde(default)]
    pub stem_html: String,
    #[serde(default)]
    pub options: Vec<OptionItem>,
    pub answer: Answer,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub paper_title: String,
}

#[derive(Deserialize)]
pub struct ImportHtmlBody {
    #[serde(default = "default_filename")]
    pub filename: String,
    pub html: String,
}

fn default_filename() -> String {
    "upload.html".into()
}

fn content_disposition(name: &str) -> axum::http::HeaderValue {
    let safe: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();
    let file = format!("{safe}.html");
    let mut encoded = String::new();
    for b in file.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' => encoded.push(*b as char),
            _ => encoded.push_str(&format!("%{b:02X}")),
        }
    }
    let value = format!("attachment; filename=\"bank.html\"; filename*=UTF-8''{encoded}");
    value
        .parse()
        .unwrap_or_else(|_| "attachment; filename=\"bank.html\"".parse().unwrap())
}

#[derive(Serialize)]
pub struct ImportResult {
    pub imported: usize,
    pub failed: usize,
    pub failures: Vec<bank_html::ImportFailure>,
}

pub async fn list_banks(State(state): State<AppState>) -> Result<Json<BankList>, AppError> {
    let rows = sqlx::query(
        r#"SELECT b.id, b.name, b.created_at, b.updated_at,
                  (SELECT COUNT(*) FROM questions q WHERE q.bank_id = b.id) AS question_count
           FROM banks b ORDER BY b.updated_at DESC"#,
    )
    .fetch_all(&state.pool)
    .await?;
    let banks = rows
        .into_iter()
        .map(|r| BankSummary {
            id: r.get("id"),
            name: r.get("name"),
            question_count: r.get("question_count"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect();
    Ok(Json(BankList { banks }))
}

pub async fn create_bank(
    State(state): State<AppState>,
    Json(body): Json<CreateBankBody>,
) -> Result<(StatusCode, Json<BankSummary>), AppError> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "题库名称不能为空",
        ));
    }
    let id = Ulid::new().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO banks(id, name, created_at, updated_at) VALUES(?, ?, ?, ?)")
        .bind(&id)
        .bind(name)
        .bind(&now)
        .bind(&now)
        .execute(&state.pool)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(BankSummary {
            id,
            name: name.to_string(),
            question_count: 0,
            created_at: now.clone(),
            updated_at: now,
        }),
    ))
}

pub async fn get_bank(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<BankDetail>, AppError> {
    let row = sqlx::query("SELECT id, name, created_at, updated_at FROM banks WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "题库不存在"))?;
    let questions = load_questions(&state, &id).await?;
    Ok(Json(BankDetail {
        id: row.get("id"),
        name: row.get("name"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        questions,
    }))
}

pub async fn rename_bank(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateBankBody>,
) -> Result<StatusCode, AppError> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "题库名称不能为空",
        ));
    }
    let now = Utc::now().to_rfc3339();
    let n = sqlx::query("UPDATE banks SET name = ?, updated_at = ? WHERE id = ?")
        .bind(name)
        .bind(&now)
        .bind(&id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "题库不存在"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_bank(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    ensure_bank(&state, &id).await?;
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM questions WHERE bank_id = ?")
            .bind(&id)
            .fetch_all(&state.pool)
            .await?;
    for qid in &ids {
        if question_locked(&state, qid).await? {
            return Err(locked_error());
        }
    }
    sqlx::query("DELETE FROM questions WHERE bank_id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM banks WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_question(
    State(state): State<AppState>,
    Path(bank_id): Path<String>,
    Json(body): Json<QuestionBody>,
) -> Result<(StatusCode, Json<StoredQuestion>), AppError> {
    ensure_bank(&state, &bank_id).await?;
    validate_question(&body)?;
    let id = Ulid::new().to_string();
    insert_question(&state, &id, &bank_id, &body, "").await?;
    touch_bank(&state, &bank_id).await?;
    Ok((
        StatusCode::CREATED,
        Json(StoredQuestion {
            id,
            bank_id,
            qtype: body.qtype,
            stem: body.stem,
            stem_html: body.stem_html,
            options: body.options,
            answer: body.answer,
            source: body.source,
            paper_title: body.paper_title,
            imported_from: String::new(),
        }),
    ))
}

pub async fn update_question(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<QuestionBody>,
) -> Result<StatusCode, AppError> {
    validate_question(&body)?;
    let bank_id: String = sqlx::query_scalar("SELECT bank_id FROM questions WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "题目不存在"))?;
    let options = serde_json::to_string(&body.options).unwrap_or_else(|_| "[]".into());
    let answer = serde_json::to_string(&body.answer).unwrap_or_else(|_| "{}".into());
    sqlx::query(
        r#"UPDATE questions SET qtype=?, stem=?, stem_html=?, options_json=?, answer_json=?,
           source=?, paper_title=? WHERE id=?"#,
    )
    .bind(qtype_str(body.qtype))
    .bind(&body.stem)
    .bind(&body.stem_html)
    .bind(&options)
    .bind(&answer)
    .bind(&body.source)
    .bind(&body.paper_title)
    .bind(&id)
    .execute(&state.pool)
    .await?;
    touch_bank(&state, &bank_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_question(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let bank_id: String = sqlx::query_scalar("SELECT bank_id FROM questions WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "题目不存在"))?;
    if question_locked(&state, &id).await? {
        return Err(locked_error());
    }
    sqlx::query("DELETE FROM questions WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await?;
    touch_bank(&state, &bank_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn import_html(
    State(state): State<AppState>,
    Path(bank_id): Path<String>,
    Json(body): Json<ImportHtmlBody>,
) -> Result<Json<ImportResult>, AppError> {
    ensure_bank(&state, &bank_id).await?;
    let report = parse_html(&body.html, &body.filename).map_err(|e| {
        AppError::new(StatusCode::BAD_REQUEST, "PARSE_FAILED", e.to_string()).with_details(json!({
            "filename": body.filename,
        }))
    })?;
    for q in &report.success {
        let id = Ulid::new().to_string();
        let body = QuestionBody {
            qtype: q.qtype,
            stem: q.stem.clone(),
            stem_html: q.stem_html.clone(),
            options: q.options.clone(),
            answer: q.answer.clone(),
            source: q.source.clone(),
            paper_title: q.paper_title.clone(),
        };
        insert_question(&state, &id, &bank_id, &body, &report.filename).await?;
    }
    touch_bank(&state, &bank_id).await?;
    Ok(Json(ImportResult {
        imported: report.success.len(),
        failed: report.failures.len(),
        failures: report.failures,
    }))
}

pub async fn export_html(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(HeaderMap, String), AppError> {
    let name: String = sqlx::query_scalar("SELECT name FROM banks WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "题库不存在"))?;
    let stored = load_questions(&state, &id).await?;
    let questions: Vec<Question> = stored
        .into_iter()
        .map(|q| Question {
            qtype: q.qtype,
            stem: q.stem,
            stem_html: q.stem_html,
            options: q.options,
            answer: q.answer,
            source: q.source,
            paper_title: q.paper_title,
        })
        .collect();
    let html = pretty_print(&name, &questions);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/html; charset=utf-8".parse().unwrap(),
    );
    headers.insert(header::CONTENT_DISPOSITION, content_disposition(&name));
    Ok((headers, html))
}

async fn ensure_bank(state: &AppState, id: &str) -> Result<(), AppError> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM banks WHERE id = ?")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if n == 0 {
        return Err(AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "题库不存在"));
    }
    Ok(())
}

async fn touch_bank(state: &AppState, id: &str) -> Result<(), AppError> {
    sqlx::query("UPDATE banks SET updated_at = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

async fn insert_question(
    state: &AppState,
    id: &str,
    bank_id: &str,
    body: &QuestionBody,
    imported_from: &str,
) -> Result<(), AppError> {
    let options = serde_json::to_string(&body.options).unwrap_or_else(|_| "[]".into());
    let answer = serde_json::to_string(&body.answer).unwrap_or_else(|_| "{}".into());
    sqlx::query(
        r#"INSERT INTO questions(id, bank_id, qtype, stem, stem_html, options_json, answer_json,
           source, paper_title, imported_from) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(id)
    .bind(bank_id)
    .bind(qtype_str(body.qtype))
    .bind(&body.stem)
    .bind(&body.stem_html)
    .bind(&options)
    .bind(&answer)
    .bind(&body.source)
    .bind(&body.paper_title)
    .bind(imported_from)
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub(crate) async fn load_questions(state: &AppState, bank_id: &str) -> Result<Vec<StoredQuestion>, AppError> {
    let rows = sqlx::query(
        r#"SELECT id, bank_id, qtype, stem, stem_html, options_json, answer_json, source,
                  paper_title, imported_from FROM questions WHERE bank_id = ?"#,
    )
    .bind(bank_id)
    .fetch_all(&state.pool)
    .await?;
    let mut out = Vec::new();
    for r in rows {
        let qtype: String = r.get("qtype");
        let options: String = r.get("options_json");
        let answer: String = r.get("answer_json");
        out.push(StoredQuestion {
            id: r.get("id"),
            bank_id: r.get("bank_id"),
            qtype: parse_qtype(&qtype),
            stem: r.get("stem"),
            stem_html: r.get("stem_html"),
            options: serde_json::from_str(&options).unwrap_or_default(),
            answer: serde_json::from_str(&answer).unwrap_or(Answer::Fill(vec![])),
            source: r.get("source"),
            paper_title: r.get("paper_title"),
            imported_from: r.get("imported_from"),
        });
    }
    Ok(out)
}

async fn question_locked(state: &AppState, question_id: &str) -> Result<bool, AppError> {
    let needle = format!("\"question_id\":\"{question_id}\"");
    let n: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM papers p
           JOIN exam_tasks t ON t.id = p.task_id
           WHERE t.status != 'draft' AND instr(p.items_json, ?) > 0"#,
    )
    .bind(&needle)
    .fetch_one(&state.pool)
    .await?;
    Ok(n > 0)
}

fn locked_error() -> AppError {
    AppError::new(
        StatusCode::CONFLICT,
        "IN_USE",
        "该题目已被考试任务引用，不能删除",
    )
}

fn validate_question(body: &QuestionBody) -> Result<(), AppError> {
    if body.stem.trim().is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "题干不能为空",
        ));
    }
    match body.qtype {
        QType::Single | QType::Multiple if body.options.is_empty() => Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "选择题需要选项",
        )),
        _ => Ok(()),
    }
}

pub(crate) fn qtype_str(q: QType) -> &'static str {
    match q {
        QType::Single => "single",
        QType::Multiple => "multiple",
        QType::Judge => "judge",
        QType::Fill => "fill",
        QType::Code => "code",
    }
}

pub(crate) fn parse_qtype(s: &str) -> QType {
    match s {
        "multiple" => QType::Multiple,
        "judge" => QType::Judge,
        "fill" => QType::Fill,
        "code" => QType::Code,
        _ => QType::Single,
    }
}
