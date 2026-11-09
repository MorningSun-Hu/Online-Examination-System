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
    needs_manual, parse_answers, parse_items, qtype_map_to_json, task_expired, task_from_row, TaskRow,
};
use crate::students::{missing_student_ids, student_ids_for_classes};
use crate::AppState;
use paper_gen::TYPE_ORDER;

#[derive(Deserialize)]
pub struct RuleBody {
    pub bank_ids: Vec<String>,
    pub type_counts: HashMap<String, usize>,
    #[serde(default = "default_true")]
    pub shuffle_within_section: bool,
    #[serde(default = "default_true")]
    pub shuffle_options: bool,
    pub type_subtotals: Option<HashMap<String, u32>>,
    #[serde(default)]
    pub bank_draws: Vec<crate::paper::BankDraw>,
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
    pub total_score: Option<i64>,
    pub pass_score: Option<i64>,
    #[serde(default)]
    pub student_ids: Vec<String>,
    #[serde(default)]
    pub class_names: Vec<String>,
    pub rule: Option<RuleBody>,
    pub paper_id: Option<String>,
    pub parent_id: Option<String>,
    pub starts_at: Option<String>,
    #[serde(default)]
    pub auto_grade: Option<bool>,
}

#[derive(Deserialize)]
pub struct PatchTaskBody {
    pub allow_early_submit: Option<bool>,
    pub score_plan: Option<HashMap<String, Vec<u32>>>,
    pub total_score: Option<i64>,
    pub pass_score: Option<i64>,
    #[serde(default)]
    pub auto_grade: Option<bool>,
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
    pub paper_id: Option<String>,
    pub parent_id: Option<String>,
    pub starts_at: Option<String>,
    pub student_path: String,
    pub auto_grade: bool,
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
        paper_id: t.paper_id.clone(),
        parent_id: t.parent_id.clone(),
        starts_at: t.starts_at.clone(),
        student_path: format!("/student.html?t={}", t.id),
        auto_grade: t.auto_grade,
    }
}

pub async fn list_tasks(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let rows = sqlx::query(
        "SELECT id, title, mode, status, duration_minutes, allow_early_submit, total_score, pass_score, published_at, closes_at, paper_id, parent_id, starts_at, auto_grade FROM exam_tasks ORDER BY id DESC",
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
                "paper_id": t.paper_id,
                "parent_id": t.parent_id,
                "starts_at": t.starts_at,
                "student_path": format!("/student.html?t={}", t.id),
                "auto_grade": t.auto_grade,
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
    if body.duration_minutes <= 0 {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "BAD_REQUEST", "时长必须为正整数"));
    }
    let mut student_ids = body.student_ids.clone();
    if !body.class_names.is_empty() {
        student_ids.extend(student_ids_for_classes(&state, &body.class_names).await?);
    }
    student_ids.sort();
    student_ids.dedup();
    let missing = missing_student_ids(&state, &student_ids).await?;
    if !missing.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "UNKNOWN_STUDENT",
            "只能从已导入名册中选择学生",
        )
        .with_details(json!({ "ids": missing })));
    }
    let paper_id = body.paper_id.clone();
    let (total, pass, paper_title, paper_auto) = if let Some(pid) = paper_id.as_deref() {
        let (p_total, p_pass, p_title, p_auto) = crate::papers::paper_totals(&state, pid).await?;
        let total = body.total_score.unwrap_or(p_total);
        let pass = body.pass_score.unwrap_or(p_pass);
        (total, pass, Some(p_title), p_auto)
    } else {
        let total = body.total_score.unwrap_or(0);
        if total <= 0 {
            return Err(AppError::new(StatusCode::BAD_REQUEST, "BAD_REQUEST", "总分必须为正整数"));
        }
        let pass = body.pass_score.unwrap_or(total * 60 / 100);
        let from_rule = true;
        (total, pass, None, from_rule)
    };
    let title = if body.title.trim().is_empty() {
        paper_title.unwrap_or_else(|| "未命名任务".into())
    } else {
        body.title.trim().to_string()
    };
    let id = Ulid::new().to_string();
    let auto_grade = if let Some(v) = body.auto_grade {
        v
    } else if let Some(parent) = body.parent_id.as_deref() {
        load_task(&state, parent).await?.auto_grade
    } else {
        paper_auto
    };
    sqlx::query(
        r#"INSERT INTO exam_tasks(id, title, mode, status, duration_minutes, allow_early_submit, total_score, pass_score, paper_id, parent_id, starts_at, auto_grade)
           VALUES(?, ?, ?, 'draft', ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(&id)
    .bind(&title)
    .bind(&body.mode)
    .bind(body.duration_minutes)
    .bind(if body.allow_early_submit { 1 } else { 0 })
    .bind(total)
    .bind(pass)
    .bind(&paper_id)
    .bind(&body.parent_id)
    .bind(&body.starts_at)
    .bind(if auto_grade { 1 } else { 0 })
    .execute(&state.pool)
    .await?;
    if let Some(pid) = paper_id.as_deref() {
        crate::papers::copy_paper_rules(&state, pid, &id).await?;
    } else if let Some(rule) = body.rule.as_ref() {
        let counts_json = serde_json::to_string(&rule.type_counts).unwrap_or_else(|_| "{}".into());
        let banks_json = serde_json::to_string(&rule.bank_ids).unwrap_or_else(|_| "[]".into());
        let sub_json = rule
            .type_subtotals
            .as_ref()
            .map(|s| serde_json::to_string(s).unwrap_or_else(|_| "{}".into()));
        sqlx::query(
            r#"INSERT INTO paper_rules(task_id, bank_ids_json, type_counts_json, shuffle_within_section, shuffle_options, type_subtotals_json, score_plan_json, bank_draws_json)
               VALUES(?, ?, ?, ?, ?, ?, NULL, ?)"#,
        )
        .bind(&id)
        .bind(&banks_json)
        .bind(&counts_json)
        .bind(if rule.shuffle_within_section { 1 } else { 0 })
        .bind(if rule.shuffle_options { 1 } else { 0 })
        .bind(&sub_json)
        .bind(serde_json::to_string(&rule.bank_draws).unwrap_or_else(|_| "[]".into()))
        .execute(&state.pool)
        .await?;
    } else if let Some(parent) = body.parent_id.as_deref() {
        clone_rules(&state, parent, &id).await?;
    } else {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "请选择已组好的试卷，或提供组卷规则",
        ));
    }
    for sid in &student_ids {
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
    match compute_score_plan(&rule, total as u32) {
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

async fn remove_draft_task(state: &AppState, id: &str) -> Result<(), AppError> {
    let task = load_task(state, id).await?;
    if task.status != "draft" {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "BAD_STATE",
            "只有草稿任务可以删除",
        ));
    }
    let paper_n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM papers WHERE task_id = ?")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if paper_n > 0 {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "HAS_ATTEMPTS",
            "已有学生作答记录，不能删除",
        ));
    }
    sqlx::query("UPDATE exam_tasks SET parent_id = NULL WHERE parent_id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM paper_rules WHERE task_id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM task_roster WHERE task_id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE task_id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM exam_tasks WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

async fn apply_score_patch(
    state: &AppState,
    task: &TaskRow,
    body: &PatchTaskBody,
) -> Result<(), AppError> {
    if task.status != "draft" {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "BAD_STATE",
            "发布后不能再改分值",
        ));
    }
    let rule = load_rule(state, &task.id).await?;
    if let Some(raw) = &body.score_plan {
        let mut grouped: HashMap<bank_html::QType, Vec<u32>> = HashMap::new();
        for qtype in TYPE_ORDER {
            let n = rule.type_counts.get(&qtype).copied().unwrap_or(0);
            if n == 0 {
                continue;
            }
            let key = crate::banks::qtype_str(qtype);
            let pts = raw.get(key).cloned().unwrap_or_default();
            if pts.len() != n {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "BAD_PLAN",
                    "分值条目数量必须与该题型题量一致",
                ));
            }
            if pts.iter().any(|&p| p == 0) {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "BAD_PLAN",
                    "每题至少 1 分",
                ));
            }
            grouped.insert(qtype, pts);
        }
        let sum: u32 = grouped.values().flatten().copied().sum();
        if (sum as usize) < grouped.values().map(|v| v.len()).sum::<usize>() {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "BAD_PLAN",
                "总分必须大于等于题量",
            ));
        }
        let total = sum as i64;
        let pass = body
            .pass_score
            .unwrap_or(task.pass_score)
            .clamp(0, total);
        let mut subtotals: HashMap<bank_html::QType, u32> = HashMap::new();
        for (k, v) in &grouped {
            subtotals.insert(*k, v.iter().sum());
        }
        let plan_json = qtype_map_to_json(&grouped);
        let sub_json = qtype_map_to_json(&subtotals);
        sqlx::query("UPDATE paper_rules SET score_plan_json = ?, type_subtotals_json = ? WHERE task_id = ?")
            .bind(plan_json.to_string())
            .bind(sub_json.to_string())
            .bind(&task.id)
            .execute(&state.pool)
            .await?;
        sqlx::query("UPDATE exam_tasks SET total_score = ?, pass_score = ? WHERE id = ?")
            .bind(total)
            .bind(pass)
            .bind(&task.id)
            .execute(&state.pool)
            .await?;
        return Ok(());
    }
    if let Some(total) = body.total_score {
        if total <= 0 {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "总分必须为正整数",
            ));
        }
        let plan = compute_score_plan(&rule, total as u32)?;
        let plan_json = qtype_map_to_json(&plan);
        sqlx::query("UPDATE paper_rules SET score_plan_json = ? WHERE task_id = ?")
            .bind(plan_json.to_string())
            .bind(&task.id)
            .execute(&state.pool)
            .await?;
        let pass = body.pass_score.unwrap_or(total * 60 / 100).clamp(0, total);
        sqlx::query("UPDATE exam_tasks SET total_score = ?, pass_score = ? WHERE id = ?")
            .bind(total)
            .bind(pass)
            .bind(&task.id)
            .execute(&state.pool)
            .await?;
        return Ok(());
    }
    if let Some(pass) = body.pass_score {
        sqlx::query("UPDATE exam_tasks SET pass_score = ? WHERE id = ?")
            .bind(pass.clamp(0, task.total_score))
            .bind(&task.id)
            .execute(&state.pool)
            .await?;
    }
    Ok(())
}

pub async fn patch_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PatchTaskBody>,
) -> Result<StatusCode, AppError> {
    let task = load_task(&state, &id).await?;
    if let Some(v) = body.allow_early_submit {
        sqlx::query("UPDATE exam_tasks SET allow_early_submit = ? WHERE id = ?")
            .bind(if v { 1 } else { 0 })
            .bind(&id)
            .execute(&state.pool)
            .await?;
    }
    if let Some(v) = body.auto_grade {
        if task.status == "released" {
            return Err(AppError::new(
                StatusCode::CONFLICT,
                "BAD_STATE",
                "试卷已公开，不能再改批改方式",
            ));
        }
        sqlx::query("UPDATE exam_tasks SET auto_grade = ? WHERE id = ?")
            .bind(if v { 1 } else { 0 })
            .bind(&id)
            .execute(&state.pool)
            .await?;
        regrade_submitted(&state, &id).await?;
    }
    if body.score_plan.is_some() || body.total_score.is_some() || body.pass_score.is_some() {
        apply_score_patch(&state, &task, &body).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    remove_draft_task(&state, &id).await?;
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
    collect_task(&state, &id).await?;
    let task = load_task(&state, &id).await?;
    let rule = load_rule(&state, &id).await.ok();
    let plan = rule
        .and_then(|r| r.score_plan)
        .as_ref()
        .map(qtype_map_to_json)
        .unwrap_or(Value::Null);
    Ok(Json(to_out(&task, plan)))
}

pub async fn end(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TaskOut>, AppError> {
    end_task(&state, &id).await?;
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
        return Err(AppError::new(StatusCode::CONFLICT, "BAD_STATE", "结束后才能提交批改"));
    }
    if pending_code(&state, &id).await? {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "PENDING_CODE",
            "仍有学生未确认批改",
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
    if task.status != "closed" && task.status != "graded" {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "BAD_STATE",
            if task.status == "published" || task.status == "draft" {
                "结束后才能公开试卷"
            } else {
                "批改完成后才能公开试卷"
            },
        ));
    }
    if pending_code(&state, &id).await? {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "PENDING_CODE",
            "还有学生未确认批改，不能公开试卷",
        ));
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
            "auto_grade": task.auto_grade,
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
        submit_in_progress(state, id).await?;
    }
    Ok(())
}

pub async fn collect_task(state: &AppState, id: &str) -> Result<(), AppError> {
    let task = load_task(state, id).await?;
    if task.status != "published" {
        return Ok(());
    }
    submit_in_progress(state, id).await
}

pub async fn end_task(state: &AppState, id: &str) -> Result<(), AppError> {
    let task = load_task(state, id).await?;
    if task.status != "published" {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "BAD_STATE",
            "只有进行中的任务可以结束",
        ));
    }
    submit_in_progress(state, id).await?;
    sqlx::query("UPDATE exam_tasks SET status=? WHERE id=?")
        .bind("closed")
        .bind(id)
        .execute(&state.pool)
        .await?;
    maybe_mark_graded(state, id).await?;
    Ok(())
}

async fn submit_in_progress(state: &AppState, id: &str) -> Result<(), AppError> {
    let task = load_task(state, id).await?;
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
        let (obj, total, _) = grade_blob(&items, &mut blob, task.auto_grade);
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
    Ok(())
}

async fn pending_code(state: &AppState, task_id: &str) -> Result<bool, AppError> {
    let task = load_task(state, task_id).await?;
    let rows = sqlx::query(
        "SELECT p.items_json, a.answers_json FROM papers p JOIN attempts a ON a.paper_id=p.id WHERE p.task_id=? AND a.status='submitted'",
    )
    .bind(task_id)
    .fetch_all(&state.pool)
    .await?;
    for row in rows {
        let items = parse_items(&row.get::<String, _>("items_json"));
        let blob = parse_answers(&row.get::<String, _>("answers_json"));
        let mut has_manual = false;
        for it in &items {
            if needs_manual(it, task.auto_grade) {
                has_manual = true;
                if !blob.code_scores.contains_key(&it.question_id) {
                    return Ok(true);
                }
            }
        }
        if has_manual && !blob.marking_confirmed {
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn maybe_mark_graded(state: &AppState, id: &str) -> Result<(), AppError> {
    let task = load_task(state, id).await?;
    if task.status != "closed" && task.status != "graded" {
        return Ok(());
    }
    let pending = pending_code(state, id).await?;
    let next = if pending { "closed" } else { "graded" };
    if task.status != next {
        sqlx::query("UPDATE exam_tasks SET status=? WHERE id=?")
            .bind(next)
            .bind(id)
            .execute(&state.pool)
            .await?;
    }
    Ok(())
}

async fn regrade_submitted(state: &AppState, task_id: &str) -> Result<(), AppError> {
    let task = load_task(state, task_id).await?;
    let rows = sqlx::query(
        "SELECT a.id, p.items_json, a.answers_json FROM papers p JOIN attempts a ON a.paper_id=p.id WHERE p.task_id=? AND a.status='submitted'",
    )
    .bind(task_id)
    .fetch_all(&state.pool)
    .await?;
    for row in rows {
        let items = parse_items(&row.get::<String, _>("items_json"));
        let mut blob = parse_answers(&row.get::<String, _>("answers_json"));
        blob.marking_confirmed = false;
        let (obj, total, _) = grade_blob(&items, &mut blob, task.auto_grade);
        sqlx::query("UPDATE attempts SET answers_json=?, objective_score=?, total_score=? WHERE id=?")
            .bind(serde_json::to_string(&blob).unwrap_or_else(|_| "{}".into()))
            .bind(obj)
            .bind(total)
            .bind(row.get::<String, _>("id"))
            .execute(&state.pool)
            .await?;
    }
    maybe_mark_graded(state, task_id).await?;
    Ok(())
}

async fn clone_rules(state: &AppState, from_task: &str, to_task: &str) -> Result<(), AppError> {
    let row = sqlx::query(
        r#"SELECT bank_ids_json, type_counts_json, shuffle_within_section, shuffle_options,
                  type_subtotals_json, score_plan_json, bank_draws_json FROM paper_rules WHERE task_id = ?"#,
    )
    .bind(from_task)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "组卷规则不存在"))?;
    sqlx::query(
        r#"INSERT INTO paper_rules(task_id, bank_ids_json, type_counts_json, shuffle_within_section,
           shuffle_options, type_subtotals_json, score_plan_json, bank_draws_json)
           VALUES(?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(to_task)
    .bind(row.get::<String, _>("bank_ids_json"))
    .bind(row.get::<String, _>("type_counts_json"))
    .bind(row.get::<i64, _>("shuffle_within_section"))
    .bind(row.get::<i64, _>("shuffle_options"))
    .bind(row.get::<Option<String>, _>("type_subtotals_json"))
    .bind(row.get::<Option<String>, _>("score_plan_json"))
    .bind(row.try_get::<Option<String>, _>("bank_draws_json").ok().flatten())
    .execute(&state.pool)
    .await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct SplitBody {
    #[serde(default)]
    pub class_names: Vec<String>,
}

pub async fn split_by_class(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SplitBody>,
) -> Result<Json<Value>, AppError> {
    let parent = load_task(&state, &id).await?;
    if parent.status != "draft" {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "BAD_STATE",
            "只有草稿可以拆分子任务",
        ));
    }
    let roster = sqlx::query(
        r#"SELECT s.id, s.class_name FROM task_roster r JOIN students s ON s.id = r.student_id WHERE r.task_id = ?"#,
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await?;
    let rows = if roster.is_empty() {
        sqlx::query("SELECT id, class_name FROM students")
            .fetch_all(&state.pool)
            .await?
    } else {
        roster
    };
    let mut by_class: HashMap<String, Vec<String>> = HashMap::new();
    for r in rows {
        let class: String = r.get("class_name");
        let sid: String = r.get("id");
        by_class.entry(class).or_default().push(sid);
    }
    let names: Vec<String> = if body.class_names.is_empty() {
        let mut n: Vec<String> = by_class.keys().cloned().collect();
        n.sort();
        n
    } else {
        body.class_names
    };
    let mut created = Vec::new();
    for class in names {
        let Some(ids) = by_class.get(&class) else {
            continue;
        };
        if ids.is_empty() {
            continue;
        }
        let child_id = Ulid::new().to_string();
        let title = format!("{} · {}", parent.title, class);
        sqlx::query(
            r#"INSERT INTO exam_tasks(id, title, mode, status, duration_minutes, allow_early_submit, total_score, pass_score, paper_id, parent_id, starts_at, auto_grade)
               VALUES(?, ?, ?, 'draft', ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(&child_id)
        .bind(&title)
        .bind(&parent.mode)
        .bind(parent.duration_minutes)
        .bind(if parent.allow_early_submit { 1 } else { 0 })
        .bind(parent.total_score)
        .bind(parent.pass_score)
        .bind(&parent.paper_id)
        .bind(&id)
        .bind(&parent.starts_at)
        .bind(if parent.auto_grade { 1 } else { 0 })
        .execute(&state.pool)
        .await?;
        clone_rules(&state, &id, &child_id).await?;
        for sid in ids {
            sqlx::query("INSERT INTO task_roster(task_id, student_id) VALUES(?, ?)")
                .bind(&child_id)
                .bind(sid)
                .execute(&state.pool)
                .await?;
        }
        created.push(json!({
            "id": child_id,
            "title": title,
            "class_name": class,
            "student_path": format!("/student.html?t={child_id}"),
        }));
    }
    if !created.is_empty() {
        remove_draft_task(&state, &id).await?;
    }
    Ok(Json(json!({ "tasks": created })))
}
