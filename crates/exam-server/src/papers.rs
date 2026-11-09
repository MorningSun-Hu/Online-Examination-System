use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use ulid::Ulid;

use crate::error::AppError;
use crate::paper::{
    check_pool, compute_score_plan, frozen_template_items, generate_snapshot, load_bank_questions,
    overlay_score_plan, qtype_map_to_json, SnapshotItem,
};
use crate::AppState;
use bank_html::QType;
use paper_gen::TYPE_ORDER;

#[derive(Debug, Clone, Deserialize)]
pub struct PaperRuleIn {
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
pub struct CreatePaperBody {
    pub title: String,
    pub total_score: i64,
    pub pass_score: Option<i64>,
    pub rule: PaperRuleIn,
    #[serde(default)]
    pub live_draw: bool,
    #[serde(default)]
    pub auto_grade: Option<bool>,
}

#[derive(Deserialize, Default)]
pub struct PatchPaperBody {
    pub score_plan: Option<HashMap<String, Vec<u32>>>,
    pub total_score: Option<i64>,
    pub pass_score: Option<i64>,
}

#[derive(Serialize)]
pub struct PaperOut {
    pub id: String,
    pub title: String,
    pub total_score: i64,
    pub pass_score: i64,
    pub bank_ids: Vec<String>,
    pub type_counts: HashMap<String, usize>,
    pub shuffle_within_section: bool,
    pub shuffle_options: bool,
    pub score_plan: Value,
    pub bank_draws: Vec<crate::paper::BankDraw>,
    pub live_draw: bool,
    pub auto_grade: bool,
    pub created_at: String,
    pub updated_at: String,
}

fn row_to_out(r: sqlx::sqlite::SqliteRow) -> PaperOut {
    let banks: Vec<String> =
        serde_json::from_str(&r.get::<String, _>("bank_ids_json")).unwrap_or_default();
    let counts: HashMap<String, usize> =
        serde_json::from_str(&r.get::<String, _>("type_counts_json")).unwrap_or_default();
    let plan: Value = r
        .get::<Option<String>, _>("score_plan_json")
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    PaperOut {
        id: r.get("id"),
        title: r.get("title"),
        total_score: r.get("total_score"),
        pass_score: r.get("pass_score"),
        bank_ids: banks,
        type_counts: counts,
        shuffle_within_section: r.get::<i64, _>("shuffle_within_section") != 0,
        shuffle_options: r.get::<i64, _>("shuffle_options") != 0,
        score_plan: plan,
        bank_draws: crate::paper::parse_bank_draws(
            r.try_get::<Option<String>, _>("bank_draws_json").ok().flatten(),
        ),
        live_draw: r.try_get::<i64, _>("live_draw").ok().unwrap_or(1) != 0,
        auto_grade: r.try_get::<i64, _>("auto_grade").ok().unwrap_or(1) != 0,
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

pub async fn list_papers(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let rows = sqlx::query(
        r#"SELECT id, title, bank_ids_json, type_counts_json, shuffle_within_section, shuffle_options,
                  type_subtotals_json, score_plan_json, bank_draws_json, live_draw, auto_grade, total_score, pass_score, created_at, updated_at
           FROM exam_papers ORDER BY updated_at DESC"#,
    )
    .fetch_all(&state.pool)
    .await?;
    let papers: Vec<PaperOut> = rows.into_iter().map(row_to_out).collect();
    Ok(Json(json!({ "papers": papers })))
}

pub async fn create_paper(
    State(state): State<AppState>,
    Json(body): Json<CreatePaperBody>,
) -> Result<(StatusCode, Json<PaperOut>), AppError> {
    if body.title.trim().is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "试卷标题不能为空",
        ));
    }
    if body.total_score <= 0 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "总分必须为正整数",
        ));
    }
    let pass = body.pass_score.unwrap_or(body.total_score * 60 / 100);
    let id = Ulid::new().to_string();
    let now = Utc::now().to_rfc3339();
    let auto_grade = body.auto_grade.unwrap_or(true);
    let banks_json = serde_json::to_string(&body.rule.bank_ids).unwrap_or_else(|_| "[]".into());
    let counts_json = serde_json::to_string(&body.rule.type_counts).unwrap_or_else(|_| "{}".into());
    let sub_json = body
        .rule
        .type_subtotals
        .as_ref()
        .map(|s| serde_json::to_string(s).unwrap_or_else(|_| "{}".into()));
    sqlx::query(
        r#"INSERT INTO exam_papers(id, title, bank_ids_json, type_counts_json, shuffle_within_section,
           shuffle_options, type_subtotals_json, bank_draws_json, score_plan_json, live_draw, auto_grade, items_json, total_score, pass_score, created_at, updated_at)
           VALUES(?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, NULL, ?, ?, ?, ?)"#,
    )
    .bind(&id)
    .bind(body.title.trim())
    .bind(&banks_json)
    .bind(&counts_json)
    .bind(if body.rule.shuffle_within_section { 1 } else { 0 })
    .bind(if body.rule.shuffle_options { 1 } else { 0 })
    .bind(&sub_json)
    .bind(serde_json::to_string(&body.rule.bank_draws).unwrap_or_else(|_| "[]".into()))
    .bind(if body.live_draw { 1 } else { 0 })
    .bind(if auto_grade { 1 } else { 0 })
    .bind(body.total_score)
    .bind(pass)
    .bind(&now)
    .bind(&now)
    .execute(&state.pool)
    .await?;
    apply_plan(&state, &id, body.total_score as u32).await?;
    if !body.live_draw {
        freeze_paper_items(&state, &id, body.total_score as u32).await?;
    }
    let paper = get_paper_row(&state, &id).await?;
    Ok((StatusCode::CREATED, Json(paper)))
}

async fn apply_plan(state: &AppState, id: &str, total: u32) -> Result<(), AppError> {
    let rule = load_paper_as_rule(state, id).await?;
    let pool = load_bank_questions(state, &rule.bank_ids).await?;
    check_pool(&pool, &rule)?;
    let plan = compute_score_plan(&rule, total)?;
    let plan_json = qtype_map_to_json(&plan);
    sqlx::query("UPDATE exam_papers SET score_plan_json = ?, updated_at = ? WHERE id = ?")
        .bind(plan_json.to_string())
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

pub async fn get_paper(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PaperOut>, AppError> {
    Ok(Json(get_paper_row(&state, &id).await?))
}

pub async fn patch_paper(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PatchPaperBody>,
) -> Result<StatusCode, AppError> {
    let paper = get_paper_row(&state, &id).await?;
    apply_paper_score_patch(&state, &paper, &body).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn preview_paper(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let paper = get_paper_row(&state, &id).await?;
    if !paper.live_draw {
        if let Some(mut items) = frozen_template_items(&state, &id).await? {
            let rule = load_paper_as_rule(&state, &id).await?;
            if let Some(plan) = &rule.score_plan {
                overlay_score_plan(&mut items, plan);
            }
            return Ok(Json(preview_json(&paper, &items)));
        }
    }
    let mut rule = load_paper_as_rule(&state, &id).await?;
    rule.shuffle_within_section = false;
    rule.shuffle_options = false;
    let pool = load_bank_questions(&state, &rule.bank_ids).await?;
    let items = generate_snapshot(&pool, &rule, paper.total_score as u32)?;
    Ok(Json(preview_json(&paper, &items)))
}

async fn get_paper_row(state: &AppState, id: &str) -> Result<PaperOut, AppError> {
    let row = sqlx::query(
        r#"SELECT id, title, bank_ids_json, type_counts_json, shuffle_within_section, shuffle_options,
                  type_subtotals_json, score_plan_json, bank_draws_json, live_draw, auto_grade, total_score, pass_score, created_at, updated_at
           FROM exam_papers WHERE id = ?"#,
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "试卷不存在"))?;
    Ok(row_to_out(row))
}

pub async fn delete_paper(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let used: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM exam_tasks WHERE paper_id = ? AND status != 'draft'",
    )
    .bind(&id)
    .fetch_one(&state.pool)
    .await?;
    if used > 0 {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "IN_USE",
            "试卷已被已发布任务使用，不能删除",
        ));
    }
    let n = sqlx::query("DELETE FROM exam_papers WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "试卷不存在",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn apply_paper_score_patch(
    state: &AppState,
    paper: &PaperOut,
    body: &PatchPaperBody,
) -> Result<(), AppError> {
    let rule = load_paper_as_rule(state, &paper.id).await?;
    if let Some(raw) = &body.score_plan {
        let mut grouped: HashMap<QType, Vec<u32>> = HashMap::new();
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
        let nq: usize = grouped.values().map(|v| v.len()).sum();
        if (sum as usize) < nq {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "BAD_PLAN",
                "总分必须大于等于题量",
            ));
        }
        let total = sum as i64;
        let pass = body.pass_score.unwrap_or(paper.pass_score).clamp(0, total);
        let mut subtotals: HashMap<QType, u32> = HashMap::new();
        for (k, v) in &grouped {
            subtotals.insert(*k, v.iter().sum());
        }
        let plan_json = qtype_map_to_json(&grouped);
        let sub_json = qtype_map_to_json(&subtotals);
        sqlx::query(
            "UPDATE exam_papers SET score_plan_json = ?, type_subtotals_json = ?, total_score = ?, pass_score = ?, updated_at = ? WHERE id = ?",
        )
        .bind(plan_json.to_string())
        .bind(sub_json.to_string())
        .bind(total)
        .bind(pass)
        .bind(Utc::now().to_rfc3339())
        .bind(&paper.id)
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
        apply_plan(state, &paper.id, total as u32).await?;
        let pass = body.pass_score.unwrap_or(total * 60 / 100).clamp(0, total);
        sqlx::query("UPDATE exam_papers SET total_score = ?, pass_score = ?, updated_at = ? WHERE id = ?")
            .bind(total)
            .bind(pass)
            .bind(Utc::now().to_rfc3339())
            .bind(&paper.id)
            .execute(&state.pool)
            .await?;
        return Ok(());
    }
    if let Some(pass) = body.pass_score {
        sqlx::query("UPDATE exam_papers SET pass_score = ?, updated_at = ? WHERE id = ?")
            .bind(pass.clamp(0, paper.total_score))
            .bind(Utc::now().to_rfc3339())
            .bind(&paper.id)
            .execute(&state.pool)
            .await?;
    }
    Ok(())
}

pub async fn load_paper_as_rule(
    state: &AppState,
    id: &str,
) -> Result<crate::paper::RuleRow, AppError> {
    let row = sqlx::query(
        r#"SELECT bank_ids_json, type_counts_json, shuffle_within_section, shuffle_options,
                  type_subtotals_json, score_plan_json, bank_draws_json FROM exam_papers WHERE id = ?"#,
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "试卷不存在"))?;
    let bank_ids: Vec<String> =
        serde_json::from_str(&row.get::<String, _>("bank_ids_json")).unwrap_or_default();
    let counts: HashMap<String, usize> =
        serde_json::from_str(&row.get::<String, _>("type_counts_json")).unwrap_or_default();
    let subtotals: Option<HashMap<String, u32>> = row
        .get::<Option<String>, _>("type_subtotals_json")
        .and_then(|s| serde_json::from_str(&s).ok());
    let plan: Option<HashMap<String, Vec<u32>>> = row
        .get::<Option<String>, _>("score_plan_json")
        .and_then(|s| serde_json::from_str(&s).ok());
    Ok(crate::paper::RuleRow {
        bank_ids,
        type_counts: crate::paper::parse_qtype_map(&counts),
        shuffle_within_section: row.get::<i64, _>("shuffle_within_section") != 0,
        shuffle_options: row.get::<i64, _>("shuffle_options") != 0,
        type_subtotals: subtotals.as_ref().map(crate::paper::parse_qtype_map),
        score_plan: plan.as_ref().map(crate::paper::parse_qtype_map),
        bank_draws: crate::paper::parse_bank_draws(
            row.try_get::<Option<String>, _>("bank_draws_json").ok().flatten(),
        ),
    })
}

pub async fn copy_paper_rules(
    state: &AppState,
    paper_id: &str,
    task_id: &str,
) -> Result<(), AppError> {
    let row = sqlx::query(
        r#"SELECT bank_ids_json, type_counts_json, shuffle_within_section, shuffle_options,
                  type_subtotals_json, score_plan_json, bank_draws_json FROM exam_papers WHERE id = ?"#,
    )
    .bind(paper_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "试卷不存在"))?;
    sqlx::query(
        r#"INSERT INTO paper_rules(task_id, bank_ids_json, type_counts_json, shuffle_within_section,
           shuffle_options, type_subtotals_json, score_plan_json, bank_draws_json)
           VALUES(?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(task_id)
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

pub async fn paper_totals(state: &AppState, id: &str) -> Result<(i64, i64, String, bool), AppError> {
    let row = sqlx::query("SELECT title, total_score, pass_score, auto_grade FROM exam_papers WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "试卷不存在"))?;
    Ok((
        row.get("total_score"),
        row.get("pass_score"),
        row.get("title"),
        row.try_get::<i64, _>("auto_grade").unwrap_or(1) != 0,
    ))
}

async fn freeze_paper_items(state: &AppState, id: &str, total: u32) -> Result<(), AppError> {
    let rule = load_paper_as_rule(state, id).await?;
    let pool = load_bank_questions(state, &rule.bank_ids).await?;
    let items = generate_snapshot(&pool, &rule, total)?;
    sqlx::query("UPDATE exam_papers SET items_json = ?, updated_at = ? WHERE id = ?")
        .bind(serde_json::to_string(&items).unwrap_or_else(|_| "[]".into()))
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

fn preview_json(paper: &PaperOut, items: &[SnapshotItem]) -> Value {
    let questions: Vec<Value> = items
        .iter()
        .map(|it| {
            json!({
                "id": it.question_id,
                "qtype": it.qtype,
                "stem": it.stem,
                "stem_html": it.stem_html,
                "options": it.options,
                "answer": it.answer,
                "points": it.points,
            })
        })
        .collect();
    json!({
        "id": paper.id,
        "title": paper.title,
        "total_score": paper.total_score,
        "pass_score": paper.pass_score,
        "score_plan": paper.score_plan,
        "live_draw": paper.live_draw,
        "questions": questions,
    })
}
