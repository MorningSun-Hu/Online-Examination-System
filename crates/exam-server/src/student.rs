use axum::extract::{Extension, Request, State};
use axum::extract::Query;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum::Json;
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use ulid::Ulid;

use crate::auth::hash_token;
use crate::db;
use crate::error::AppError;
use crate::paper::{
    exam_facing_answer, frozen_template_items, generate_snapshot, grade_blob, load_bank_questions,
    load_rule, load_task, overlay_score_plan, parse_answers, parse_items, parse_rfc3339,
    student_items, AnswersBlob, SnapshotItem, StudentAnswer, TaskRow,
};
use crate::tasks::maybe_auto_close;
use crate::AppState;

#[derive(Clone)]
pub struct StudentCtx {
    pub student_id: String,
    pub task_id: String,
    pub client_id: String,
}

#[derive(Deserialize)]
pub struct EnterBody {
    pub class_name: Option<String>,
    pub name: Option<String>,
    pub student_no: Option<String>,
    pub client_id: String,
    pub task_id: Option<String>,
}

#[derive(Deserialize)]
pub struct SaveBody {
    pub answers: std::collections::HashMap<String, StudentAnswer>,
}

pub async fn student_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = student_token(&req)?;
    let hash = hash_token(&token);
    let sess = db::student_session(&state.pool, &hash)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "DB", e.to_string()))?
        .ok_or_else(|| AppError::new(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "学生未登录"))?;
    let exp = parse_rfc3339(&sess.expires_at)
        .ok_or_else(|| AppError::new(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "学生会话无效"))?;
    if exp < Utc::now() {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "学生会话已过期",
        ));
    }
    if let Some(want) = requested_task_id(&req) {
        if want != sess.task_id {
            return Err(AppError::new(
                StatusCode::FORBIDDEN,
                "TASK_MISMATCH",
                "入口与当前答卷不是同一场考试",
            ));
        }
    }
    req.extensions_mut().insert(StudentCtx {
        student_id: sess.student_id,
        task_id: sess.task_id,
        client_id: sess.client_id,
    });
    Ok(next.run(req).await)
}

fn student_token(req: &Request) -> Result<String, AppError> {
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
            if let Some(t) = part.trim().strip_prefix("exam_student=") {
                if !t.is_empty() {
                    return Ok(t.to_string());
                }
            }
        }
    }
    Err(AppError::new(
        StatusCode::UNAUTHORIZED,
        "UNAUTHORIZED",
        "学生未登录",
    ))
}

fn requested_task_id(req: &Request) -> Option<String> {
    if let Some(v) = req
        .headers()
        .get("x-exam-task")
        .and_then(|v| v.to_str().ok())
    {
        let t = v.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    let query = req.uri().query()?;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == "t" || k == "task_id" {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

pub async fn enter(
    State(state): State<AppState>,
    Json(body): Json<EnterBody>,
) -> Result<Json<Value>, AppError> {
    let name = body.name.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let no = body
        .student_no
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if name.is_none() && no.is_none() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "姓名或学号至少填一项",
        ));
    }
    if body.client_id.trim().is_empty() {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "BAD_REQUEST", "缺少客户端标识"));
    }
    let task_id = resolve_task(&state, body.task_id.as_deref()).await?;
    maybe_auto_close(&state, &task_id).await?;
    let task = load_task(&state, &task_id).await?;
    match task.status.as_str() {
        "published" | "released" => {}
        _ => {
            return Err(AppError::new(
                StatusCode::FORBIDDEN,
                "NOT_OPEN",
                "当前没有可进入的考试任务",
            ))
        }
    }
    if let Some(start) = task.starts_at.as_deref().and_then(parse_rfc3339) {
        if start > Utc::now() {
            return Err(AppError::new(
                StatusCode::FORBIDDEN,
                "NOT_STARTED",
                "考试尚未开始",
            ));
        }
    }
    let class = body
        .class_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let student_id = match_student(&state, &task_id, class, name, no).await?;
    let profile = student_profile(&state, &student_id).await?;
    let (paper_id, attempt_id, items, blob, attempt_no, status) =
        ensure_paper(&state, &task, &student_id, &body.client_id).await?;
    db::delete_student_sessions(&state.pool, &student_id, &task_id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "DB", e.to_string()))?;
    let token = random_hex();
    let expires = (Utc::now() + Duration::hours(12)).to_rfc3339();
    db::insert_student_session(
        &state.pool,
        &hash_token(&token),
        &student_id,
        &task_id,
        &body.client_id,
        &expires,
    )
    .await
    .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "DB", e.to_string()))?;
    let hide_points = task.mode == "exam" && task.status != "released";
    Ok(Json(json!({
        "token": token,
        "expires_at": expires,
        "task_id": task_id,
        "student_id": student_id,
        "paper_id": paper_id,
        "attempt_id": attempt_id,
        "attempt_no": attempt_no,
        "status": status,
        "mode": task.mode,
        "title": task.title,
        "allow_early_submit": task.allow_early_submit,
        "closes_at": task.closes_at,
        "items": student_items(&items, hide_points),
        "answers": blob.items,
        "released": task.status == "released",
        "duration_minutes": task.duration_minutes,
        "total_score": task.total_score,
        "class_name": profile.0,
        "name": profile.1,
        "student_no": profile.2,
    })))
}

async fn resolve_task(state: &AppState, given: Option<&str>) -> Result<String, AppError> {
    if let Some(id) = given {
        return Ok(id.to_string());
    }
    let id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM exam_tasks WHERE status IN ('published','released') ORDER BY published_at DESC LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await?;
    id.ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "当前没有已发布任务"))
}

async fn candidate_students(
    state: &AppState,
    task_id: &str,
) -> Result<Vec<sqlx::sqlite::SqliteRow>, AppError> {
    let roster_n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_roster WHERE task_id = ?")
        .bind(task_id)
        .fetch_one(&state.pool)
        .await?;
    if roster_n == 0 {
        Ok(sqlx::query("SELECT id, student_no, name, class_name FROM students")
            .fetch_all(&state.pool)
            .await?)
    } else {
        Ok(sqlx::query(
            r#"SELECT s.id, s.student_no, s.name, s.class_name
               FROM task_roster r JOIN students s ON s.id = r.student_id
               WHERE r.task_id = ?"#,
        )
        .bind(task_id)
        .fetch_all(&state.pool)
        .await?)
    }
}

async fn student_profile(
    state: &AppState,
    id: &str,
) -> Result<(String, String, String), AppError> {
    let row = sqlx::query("SELECT class_name, name, student_no FROM students WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "学生不存在"))?;
    Ok((row.get("class_name"), row.get("name"), row.get("student_no")))
}

#[derive(Deserialize)]
pub struct BriefQuery {
    pub task_id: Option<String>,
    pub t: Option<String>,
}

pub async fn brief(
    State(state): State<AppState>,
    Query(q): Query<BriefQuery>,
) -> Result<Json<Value>, AppError> {
    let given = q.task_id.as_deref().or(q.t.as_deref());
    let task_id = resolve_task(&state, given).await?;
    maybe_auto_close(&state, &task_id).await?;
    let task = load_task(&state, &task_id).await?;
    let rows = candidate_students(&state, &task_id).await?;
    let mut classes: Vec<String> = rows
        .iter()
        .map(|r| r.get::<String, _>("class_name"))
        .collect();
    classes.sort();
    classes.dedup();
    Ok(Json(json!({
        "task_id": task.id,
        "title": task.title,
        "mode": task.mode,
        "status": task.status,
        "duration_minutes": task.duration_minutes,
        "total_score": task.total_score,
        "pass_score": task.pass_score,
        "closes_at": task.closes_at,
        "starts_at": task.starts_at,
        "classes": classes,
        "single_class": classes.len() == 1,
    })))
}

#[derive(Deserialize)]
pub struct LookupQuery {
    pub task_id: Option<String>,
    pub t: Option<String>,
    pub class_name: Option<String>,
    pub name: Option<String>,
    pub student_no: Option<String>,
}

pub async fn lookup(
    State(state): State<AppState>,
    Query(q): Query<LookupQuery>,
) -> Result<Json<Value>, AppError> {
    let given = q.task_id.as_deref().or(q.t.as_deref());
    let task_id = resolve_task(&state, given).await?;
    let name = q.name.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let no = q
        .student_no
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if name.is_none() && no.is_none() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "姓名或学号至少填一项",
        ));
    }
    let class = q
        .class_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let id = match_student(&state, &task_id, class, name, no).await?;
    let profile = student_profile(&state, &id).await?;
    Ok(Json(json!({
        "student_id": id,
        "class_name": profile.0,
        "name": profile.1,
        "student_no": profile.2,
    })))
}

async fn match_student(
    state: &AppState,
    task_id: &str,
    class_name: Option<&str>,
    name: Option<&str>,
    student_no: Option<&str>,
) -> Result<String, AppError> {
    let rows = candidate_students(state, task_id).await?;
    if let Some(no) = student_no {
        let hits: Vec<_> = rows
            .iter()
            .filter(|r| {
                r.get::<String, _>("student_no") == no
            })
            .collect();
        if hits.is_empty() {
            return Err(AppError::new(
                StatusCode::FORBIDDEN,
                "NO_MATCH",
                "名单中无此学生",
            ));
        }
        let row = hits[0];
        if let Some(c) = class_name {
            if row.get::<String, _>("class_name") != c {
                return Err(AppError::new(
                    StatusCode::FORBIDDEN,
                    "CLASS_MISMATCH",
                    "班级与学号不匹配",
                ));
            }
        }
        if let Some(n) = name {
            if row.get::<String, _>("name") != n {
                return Err(AppError::new(
                    StatusCode::FORBIDDEN,
                    "NAME_MISMATCH",
                    "姓名与学号不匹配",
                ));
            }
        }
        return Ok(row.get("id"));
    }
    let n = name.unwrap();
    let hits: Vec<_> = rows
        .iter()
        .filter(|r| {
            let name_ok = r.get::<String, _>("name") == n;
            let class_ok = class_name
                .map(|c| r.get::<String, _>("class_name") == c)
                .unwrap_or(true);
            name_ok && class_ok
        })
        .collect();
    match hits.len() {
        0 => Err(AppError::new(
            StatusCode::FORBIDDEN,
            "NO_MATCH",
            "名单中无此学生",
        )),
        1 => Ok(hits[0].get("id")),
        _ => Err(AppError::new(
            StatusCode::CONFLICT,
            "DUPLICATE_NAME",
            "存在重名，请改填学号",
        )),
    }
}

async fn ensure_paper(
    state: &AppState,
    task: &TaskRow,
    student_id: &str,
    client_id: &str,
) -> Result<(String, String, Vec<SnapshotItem>, AnswersBlob, i64, String), AppError> {
    let existing = sqlx::query(
        r#"SELECT p.id, p.attempt_no, p.items_json, a.id AS attempt_id, a.status, a.answers_json
           FROM papers p JOIN attempts a ON a.paper_id = p.id
           WHERE p.task_id=? AND p.student_id=?
           ORDER BY p.attempt_no DESC LIMIT 1"#,
    )
    .bind(&task.id)
    .bind(student_id)
    .fetch_optional(&state.pool)
    .await?;
    if let Some(row) = existing {
        let status: String = row.get("status");
        let attempt_no: i64 = row.get("attempt_no");
        if task.mode == "exam" || status != "submitted" {
            let paper_id: String = row.get("id");
            let attempt_id: String = row.get("attempt_id");
            sqlx::query("UPDATE attempts SET client_id=? WHERE id=?")
                .bind(client_id)
                .bind(&attempt_id)
                .execute(&state.pool)
                .await?;
            return Ok((
                paper_id,
                attempt_id,
                parse_items(&row.get::<String, _>("items_json")),
                parse_answers(&row.get::<String, _>("answers_json")),
                attempt_no,
                status,
            ));
        }
        if task.status != "published" {
            return Ok((
                row.get("id"),
                row.get("attempt_id"),
                parse_items(&row.get::<String, _>("items_json")),
                parse_answers(&row.get::<String, _>("answers_json")),
                attempt_no,
                status,
            ));
        }
        return create_paper(state, task, student_id, client_id, attempt_no + 1).await;
    }
    if task.status != "published" {
        return Err(AppError::new(
            StatusCode::FORBIDDEN,
            "NOT_OPEN",
            "没有可作答的试卷",
        ));
    }
    create_paper(state, task, student_id, client_id, 1).await
}

async fn create_paper(
    state: &AppState,
    task: &TaskRow,
    student_id: &str,
    client_id: &str,
    attempt_no: i64,
) -> Result<(String, String, Vec<SnapshotItem>, AnswersBlob, i64, String), AppError> {
    let rule = load_rule(state, &task.id).await?;
    let mut items = match task.paper_id.as_deref() {
        Some(pid) => frozen_template_items(state, pid).await?,
        None => None,
    };
    if let (Some(plan), Some(frozen)) = (rule.score_plan.as_ref(), items.as_mut()) {
        overlay_score_plan(frozen, plan);
    }
    let items = if let Some(frozen) = items {
        frozen
    } else {
        let pool = load_bank_questions(state, &rule.bank_ids).await?;
        generate_snapshot(&pool, &rule, task.total_score as u32)?
    };
    let paper_id = Ulid::new().to_string();
    let attempt_id = Ulid::new().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO papers(id, task_id, student_id, attempt_no, items_json, created_at) VALUES(?,?,?,?,?,?)",
    )
    .bind(&paper_id)
    .bind(&task.id)
    .bind(student_id)
    .bind(attempt_no)
    .bind(serde_json::to_string(&items).unwrap_or_else(|_| "[]".into()))
    .bind(&now)
    .execute(&state.pool)
    .await?;
    let blob = AnswersBlob::default();
    sqlx::query(
        "INSERT INTO attempts(id, paper_id, client_id, status, answers_json) VALUES(?,?,?,'in_progress',?)",
    )
    .bind(&attempt_id)
    .bind(&paper_id)
    .bind(client_id)
    .bind(serde_json::to_string(&blob).unwrap_or_else(|_| "{}".into()))
    .execute(&state.pool)
    .await?;
    Ok((
        paper_id,
        attempt_id,
        items,
        blob,
        attempt_no,
        "in_progress".into(),
    ))
}

async fn load_current(
    state: &AppState,
    ctx: &StudentCtx,
) -> Result<(TaskRow, String, Vec<SnapshotItem>, AnswersBlob, String), AppError> {
    maybe_auto_close(state, &ctx.task_id).await?;
    let task = load_task(state, &ctx.task_id).await?;
    let row = sqlx::query(
        r#"SELECT p.items_json, a.id AS attempt_id, a.status, a.answers_json
           FROM papers p JOIN attempts a ON a.paper_id=p.id
           WHERE p.task_id=? AND p.student_id=?
           ORDER BY p.attempt_no DESC LIMIT 1"#,
    )
    .bind(&ctx.task_id)
    .bind(&ctx.student_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "没有试卷"))?;
    Ok((
        task,
        row.get("attempt_id"),
        parse_items(&row.get::<String, _>("items_json")),
        parse_answers(&row.get::<String, _>("answers_json")),
        row.get("status"),
    ))
}

pub async fn paper(
    State(state): State<AppState>,
    Extension(ctx): Extension<StudentCtx>,
) -> Result<Json<Value>, AppError> {
    let (task, attempt_id, items, blob, status) = load_current(&state, &ctx).await?;
    let hide_points = task.mode == "exam" && task.status != "released";
    let profile = student_profile(&state, &ctx.student_id).await?;
    Ok(Json(json!({
        "attempt_id": attempt_id,
        "status": status,
        "mode": task.mode,
        "title": task.title,
        "task_id": task.id,
        "allow_early_submit": task.allow_early_submit,
        "closes_at": task.closes_at,
        "task_status": task.status,
        "items": student_items(&items, hide_points),
        "answers": blob.items,
        "released": task.status == "released",
        "duration_minutes": task.duration_minutes,
        "total_score": task.total_score,
        "class_name": profile.0,
        "name": profile.1,
        "student_no": profile.2,
    })))
}

pub async fn save_answers(
    State(state): State<AppState>,
    Extension(ctx): Extension<StudentCtx>,
    Json(body): Json<SaveBody>,
) -> Result<StatusCode, AppError> {
    let (task, attempt_id, _items, mut blob, status) = load_current(&state, &ctx).await?;
    if status != "in_progress" {
        return Err(AppError::new(StatusCode::CONFLICT, "SUBMITTED", "已提交，不能再保存"));
    }
    if task.status != "published" {
        return Err(AppError::new(StatusCode::FORBIDDEN, "NOT_OPEN", "任务已结束"));
    }
    blob.items = body.answers;
    sqlx::query("UPDATE attempts SET answers_json=? WHERE id=?")
        .bind(serde_json::to_string(&blob).unwrap_or_else(|_| "{}".into()))
        .bind(&attempt_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn submit(
    State(state): State<AppState>,
    Extension(ctx): Extension<StudentCtx>,
) -> Result<Json<Value>, AppError> {
    let (task, attempt_id, items, mut blob, status) = load_current(&state, &ctx).await?;
    if status == "submitted" {
        return Err(AppError::new(StatusCode::CONFLICT, "SUBMITTED", "已经提交过"));
    }
    let expired = task
        .closes_at
        .as_deref()
        .and_then(parse_rfc3339)
        .map(|t| Utc::now() >= t)
        .unwrap_or(false);
    if !task.allow_early_submit && !expired && task.status == "published" {
        return Err(AppError::new(
            StatusCode::FORBIDDEN,
            "EARLY_SUBMIT",
            "当前不允许提前交卷",
        ));
    }
    if task.mode == "exam" {
        let n: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*) FROM papers p JOIN attempts a ON a.paper_id=p.id
               WHERE p.task_id=? AND p.student_id=? AND a.status='submitted'"#,
        )
        .bind(&task.id)
        .bind(&ctx.student_id)
        .fetch_one(&state.pool)
        .await?;
        if n > 0 {
            return Err(AppError::new(StatusCode::CONFLICT, "SUBMITTED", "已经提交过"));
        }
    }
    let (obj, total, _) = grade_blob(&items, &mut blob);
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE attempts SET status='submitted', answers_json=?, objective_score=?, total_score=?, submitted_at=? WHERE id=?",
    )
    .bind(serde_json::to_string(&blob).unwrap_or_else(|_| "{}".into()))
    .bind(obj)
    .bind(total)
    .bind(&now)
    .bind(&attempt_id)
    .execute(&state.pool)
    .await?;
    result_payload(&task, &items, &blob, "submitted", obj, total, &ctx)
}

pub async fn result(
    State(state): State<AppState>,
    Extension(ctx): Extension<StudentCtx>,
) -> Result<Json<Value>, AppError> {
    let (task, _attempt_id, items, blob, status) = load_current(&state, &ctx).await?;
    let obj = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT objective_score FROM attempts a JOIN papers p ON p.id=a.paper_id WHERE p.task_id=? AND p.student_id=? ORDER BY p.attempt_no DESC LIMIT 1",
    )
    .bind(&ctx.task_id)
    .bind(&ctx.student_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(None);
    let total = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT total_score FROM attempts a JOIN papers p ON p.id=a.paper_id WHERE p.task_id=? AND p.student_id=? ORDER BY p.attempt_no DESC LIMIT 1",
    )
    .bind(&ctx.task_id)
    .bind(&ctx.student_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or(None);
    result_payload(&task, &items, &blob, &status, obj, total, &ctx)
}

fn result_payload(
    task: &TaskRow,
    items: &[SnapshotItem],
    blob: &AnswersBlob,
    status: &str,
    obj: Option<i64>,
    total: Option<i64>,
    ctx: &StudentCtx,
) -> Result<Json<Value>, AppError> {
    let show = task.mode == "practice" && status == "submitted" || task.status == "released";
    if !show {
        return Ok(Json(json!({
            "status": status,
            "mode": task.mode,
            "released": false,
        })));
    }
    let details: Vec<Value> = items
        .iter()
        .map(|it| {
            json!({
                "question_id": it.question_id,
                "qtype": it.qtype,
                "stem": it.stem,
                "options": it.options,
                "points": it.points,
                "answer": exam_facing_answer(&it.answer, &it.option_map),
                "response": blob.items.get(&it.question_id),
                "score": blob.item_scores.get(&it.question_id),
                "correct": blob.item_correct.get(&it.question_id),
            })
        })
        .collect();
    Ok(Json(json!({
        "status": status,
        "mode": task.mode,
        "released": task.status == "released",
        "objective_score": obj,
        "total_score": total,
        "delivery": "online",
        "client_id": ctx.client_id,
        "items": details,
    })))
}

fn random_hex() -> String {
    use rand::rngs::OsRng;
    use rand::RngCore;
    let mut buf = [0u8; 32];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}
