use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use ulid::Ulid;

use crate::error::AppError;
use crate::paper::{
    check_pool, compute_score_plan, grade_blob, load_bank_questions, load_rule, load_task,
    parse_answers, parse_items, qtype_map_to_json, task_expired, task_from_row, TaskRow,
};
use crate::students::missing_student_ids;
use crate::AppState;

#[derive(Deserialize)]
pub struct RuleBody {
    pub bank_ids: Vec<String>,
    pub type_counts: HashMap<String, usize>,
    #[serde(default = "default_true")]
    pub shuffle_within_section: bool,
    #[serde(default = "default_true")]
    pub shuffle_options: bool,
    pub type_subtotals: Option<HashMap<String, u32>>,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct CreateTaskBody {
    pub title: String,
    pub mode: String,
    pub duration_minutes: i64,
    #[serde(default)]
    pub allow_early_submit: bool,
    pub total_score: i64,
    pub pass_score: Option<i64>,
    #[serde(default)]
    pub student_ids: Vec<String>,
    pub rule: RuleBody,
}

#[derive(Deserialize)]
pub struct PatchTaskBody {
    pub allow_early_submit: Option<bool>,
}

#[derive(Serialize)]
pub struct TaskOut {
    pub id: String,
    pub title: String,
    pub mode: String,
    pub status: String,
    pub duration_minutes: i64,
    pub allow_early_submit: bool,
    pub total_score: i64,
    pub pass_score: i64,
    pub published_at: Option<String>,
    pub closes_at: Option<String>,
    pub score_plan: Value,
}

fn to_out(t: &TaskRow, score_plan: Value) -> TaskOut {
    TaskOut {
        id: t.id.clone(),
        title: t.title.clone(),
        mode: t.mode.clone(),
        status: t.status.clone(),
        duration_minutes: t.duration_minutes,
        allow_early_submit: t.allow_early_submit,
        total_score: t.total_score,
        pass_score: t.pass_score,
        published_at: t.published_at.clone(),
        closes_at: t.closes_at.clone(),
        score_plan,
    }
}

pub async fn list_tasks(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let rows = sqlx::query(
        "SELECT id, title, mode, status, duration_minutes, allow_early_submit, total_score, pass_score, published_at, closes_at FROM exam_tasks ORDER BY id DESC",
    )
    .fetch_all(&state.pool)
    .await?;
    let tasks: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            let t = task_from_row(r);
            json!({
                "id": t.id,
                "title": t.title,
                "mode": t.mode,
                "status": t.status,
                "duration_minutes": t.duration_minutes,
                "allow_early_submit": t.allow_early_submit,
                "total_score": t.total_score,
                "pass_score": t.pass_score,
                "published_at": t.published_at,
                "closes_at": t.closes_at,
            })
        })
        .collect();
    Ok(Json(json!({ "tasks": tasks })))
}

pub async fn create_task(
    State(state): State<AppState>,
    Json(body): Json<CreateTaskBody>,
) -> Result<(StatusCode, Json<TaskOut>), AppError> {
    if body.title.trim().is_empty() {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "BAD_REQUEST", "标题不能为空"));
    }
    if body.mode != "exam" && body.mode != "practice" {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "BAD_REQUEST", "模式必须是 exam 或 practice"));
    }
    if body.duration_minutes <= 0 || body.total_score <= 0 {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "BAD_REQUEST", "时长与总分必须为正整数"));
    }
    let missing = missing_student_ids(&state, &body.student_ids).await?;
    if !missing.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "UNKNOWN_STUDENT",
            "只能从已导入名册中选择学生",
        )
        .with_details(json!({ "ids": missing })));
    }
    let pass = body
        .pass_score
        .unwrap_or(body.total_score * 60 / 100);
    let id = Ulid::new().to_string();
    sqlx::query(
        r#"INSERT INTO exam_tasks(id, title, mode, status, duration_minutes, allow_early_submit, total_score, pass_score)
           VALUES(?, ?, ?, 'draft', ?, ?, ?, ?)"#,
    )
    .bind(&id)
    .bind(&body.title)
    .bind(&body.mode)
    .bind(body.duration_minutes)
    .bind(if body.allow_early_submit { 1 } else { 0 })
    .bind(body.total_score)
    .bind(pass)
    .execute(&state.pool)
    .await?;
    let counts_json = serde_json::to_string(&body.rule.type_counts).unwrap_or_else(|_| "{}".into());
    let banks_json = serde_json::to_string(&body.rule.bank_ids).unwrap_or_else(|_| "[]".into());
    let sub_json = body
        .rule
        .type_subtotals
        .as_ref()
        .map(|s| serde_json::to_string(s).unwrap_or_else(|_| "{}".into()));
    sqlx::query(
        r#"INSERT INTO paper_rules(task_id, bank_ids_json, type_counts_json, shuffle_within_section, shuffle_options, type_subtotals_json, score_plan_json)
           VALUES(?, ?, ?, ?, ?, ?, NULL)"#,
    )
    .bind(&id)
    .bind(&banks_json)
    .bind(&counts_json)
    .bind(if body.rule.shuffle_within_section { 1 } else { 0 })
    .bind(if body.rule.shuffle_options { 1 } else { 0 })
    .bind(&sub_json)
    .execute(&state.pool)
    .await?;
    for sid in &body.student_ids {
        sqlx::query("INSERT INTO task_roster(task_id, student_id) VALUES(?, ?)")
            .bind(&id)
            .bind(sid)
            .execute(&state.pool)
            .await?;
    }
    let rule = crate::paper::load_rule(&state, &id).await?;
    let pool = load_bank_questions(&state, &rule.bank_ids).await?;
    if let Err(mut e) = check_pool(&pool, &rule) {
        let mut details = e.details.clone();
        if let Value::Object(ref mut m) = details {
            m.insert("task_id".into(), json!(id));
            m.insert("status".into(), json!("draft"));
        }
        e.details = details;
        return Err(e);
    }
    match compute_score_plan(&rule, body.total_score as u32) {
        Ok(plan) => {
            let plan_json = qtype_map_to_json(&plan);
            sqlx::query("UPDATE paper_rules SET score_plan_json = ? WHERE task_id = ?")
                .bind(plan_json.to_string())
                .bind(&id)
                .execute(&state.pool)
                .await?;
            let task = load_task(&state, &id).await?;
            Ok((StatusCode::CREATED, Json(to_out(&task, plan_json))))
        }
        Err(e) => Err(e.with_details(json!({"task_id": id, "status": "draft"}))),
    }
}

pub async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TaskOut>, AppError> {
    maybe_auto_close(&state, &id).await?;
    let task = load_task(&state, &id).await?;
    let rule = load_rule(&state, &id).await?;
    let plan = rule
        .score_plan
        .as_ref()
        .map(qtype_map_to_json)
        .unwrap_or(Value::Null);
    Ok(Json(to_out(&task, plan)))
}

pub async fn patch_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PatchTaskBody>,
) -> Result<StatusCode, AppError> {
    let _ = load_task(&state, &id).await?;
    if let Some(v) = body.allow_early_submit {
        sqlx::query("UPDATE exam_tasks SET allow_early_submit = ? WHERE id = ?")
            .bind(if v { 1 } else { 0 })
            .bind(&id)
            .execute(&state.pool)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn publish(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TaskOut>, AppError> {
    let task = load_task(&state, &id).await?;
    if task.status != "draft" {
        return Err(AppError::new(StatusCode::CONFLICT, "BAD_STATE", "只有草稿可以发布"));
    }
    let rule = load_rule(&state, &id).await?;
    if rule.score_plan.is_none() {
        return Err(AppError::new(StatusCode::CONFLICT, "NO_PLAN", "请先确认组卷分值"));
    }
    let pool = load_bank_questions(&state, &rule.bank_ids).await?;
    check_pool(&pool, &rule)?;
    let now = Utc::now();
    let closes = now + Duration::minutes(task.duration_minutes);
    sqlx::query(
        "UPDATE exam_tasks SET status='published', published_at=?, closes_at=? WHERE id=?",
    )
    .bind(now.to_rfc3339())
    .bind(closes.to_rfc3339())
    .bind(&id)
    .execute(&state.pool)
    .await?;
    let task = load_task(&state, &id).await?;
    Ok(Json(to_out(
        &task,
        qtype_map_to_json(rule.score_plan.as_ref().unwrap()),
    )))
}

pub async fn close(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TaskOut>, AppError> {
    close_task(&state, &id).await?;
    let task = load_task(&state, &id).await?;
    let rule = load_rule(&state, &id).await.ok();
    let plan = rule
        .and_then(|r| r.score_plan)
        .as_ref()
        .map(qtype_map_to_json)
        .unwrap_or(Value::Null);
    Ok(Json(to_out(&task, plan)))
}

pub async fn confirm_grade(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TaskOut>, AppError> {
    let task = load_task(&state, &id).await?;
    if task.status != "closed" && task.status != "graded" {
        return Err(AppError::new(StatusCode::CONFLICT, "BAD_STATE", "收卷后才能确认批改"));
    }
    if pending_code(&state, &id).await? {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "PENDING_CODE",
            "仍有编程题待给分",
        ));
    }
    sqlx::query("UPDATE exam_tasks SET status='graded' WHERE id=?")
        .bind(&id)
        .execute(&state.pool)
        .await?;
    get_task(State(state), Path(id)).await
}

pub async fn release(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let task = load_task(&state, &id).await?;
    if task.status != "graded" {
        return Err(AppError::new(StatusCode::CONFLICT, "BAD_STATE", "批改完成后才能公开试卷"));
    }
    sqlx::query("UPDATE exam_tasks SET status='released' WHERE id=?")
        .bind(&id)
        .execute(&state.pool)
        .await?;
    let online = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sessions WHERE role='student' AND task_id=? AND expires_at > ?",
    )
    .bind(&id)
    .bind(Utc::now().to_rfc3339())
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({
        "status": "released",
        "online_sessions": online,
    })))
}

pub async fn monitor(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    maybe_auto_close(&state, &id).await?;
    let task = load_task(&state, &id).await?;
    let rows = sqlx::query(
        r#"SELECT s.id, s.student_no, s.name, s.class_name,
                  (SELECT a.status FROM papers p JOIN attempts a ON a.paper_id = p.id
                   WHERE p.task_id = r.task_id AND p.student_id = s.id
                   ORDER BY p.attempt_no DESC LIMIT 1) AS attempt_status
           FROM task_roster r
           JOIN students s ON s.id = r.student_id
           WHERE r.task_id = ?
           ORDER BY s.class_name, s.student_no"#,
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await?;
    let students: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            let st: Option<String> = r.get("attempt_status");
            let status = match st.as_deref() {
                Some("submitted") => "submitted",
                Some("in_progress") => "in_progress",
                _ => "not_entered",
            };
            json!({
                "student_id": r.get::<String, _>("id"),
                "student_no": r.get::<String, _>("student_no"),
                "name": r.get::<String, _>("name"),
                "class_name": r.get::<String, _>("class_name"),
                "status": status,
            })
        })
        .collect();
    Ok(Json(json!({
        "task": {
            "id": task.id,
            "title": task.title,
            "mode": task.mode,
            "status": task.status,
            "allow_early_submit": task.allow_early_submit,
            "closes_at": task.closes_at,
        },
        "students": students,
    })))
}

pub async fn maybe_auto_close(state: &AppState, id: &str) -> Result<(), AppError> {
    let task = match load_task(state, id).await {
        Ok(t) => t,
        Err(e) if e.status == StatusCode::NOT_FOUND => return Ok(()),
        Err(e) => return Err(e),
    };
    if task_expired(&task) {
        close_task(state, id).await?;
    }
    Ok(())
}

pub async fn close_task(state: &AppState, id: &str) -> Result<(), AppError> {
    let task = load_task(state, id).await?;
    if task.status != "published" {
        return Ok(());
    }
    let papers = sqlx::query(
        "SELECT p.id, p.items_json, a.id AS attempt_id, a.status, a.answers_json
         FROM papers p JOIN attempts a ON a.paper_id = p.id
         WHERE p.task_id = ? AND a.status = 'in_progress'",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let now = Utc::now().to_rfc3339();
    for row in papers {
        let items = parse_items(&row.get::<String, _>("items_json"));
        let mut blob = parse_answers(&row.get::<String, _>("answers_json"));
        let (obj, total, _) = grade_blob(&items, &mut blob);
        let aid: String = row.get("attempt_id");
        sqlx::query(
            "UPDATE attempts SET status='submitted', answers_json=?, objective_score=?, total_score=?, submitted_at=? WHERE id=?",
        )
        .bind(serde_json::to_string(&blob).unwrap_or_else(|_| "{}".into()))
        .bind(obj)
        .bind(total)
        .bind(&now)
        .bind(&aid)
        .execute(&state.pool)
        .await?;
    }
    let next = if pending_code(state, id).await? {
        "closed"
    } else {
        "graded"
    };
    sqlx::query("UPDATE exam_tasks SET status=? WHERE id=?")
        .bind(next)
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

async fn pending_code(state: &AppState, task_id: &str) -> Result<bool, AppError> {
    let rows = sqlx::query(
        "SELECT p.items_json, a.answers_json FROM papers p JOIN attempts a ON a.paper_id=p.id WHERE p.task_id=? AND a.status='submitted'",
    )
    .bind(task_id)
    .fetch_all(&state.pool)
    .await?;
    for row in rows {
        let items = parse_items(&row.get::<String, _>("items_json"));
        let blob = parse_answers(&row.get::<String, _>("answers_json"));
        for it in items {
            if it.qtype == bank_html::QType::Code && !blob.code_scores.contains_key(&it.question_id) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
