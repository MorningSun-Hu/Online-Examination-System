use std::collections::HashMap;

use bank_html::{Answer, OptionItem, QType, Question};
use chrono::{DateTime, Utc};
use grading::{grade, StudentResponse};
use paper_gen::{allocate_scores, generate, restore_original_keys, GenError, PaperRule, TYPE_ORDER};
use rand::rngs::StdRng;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;

use crate::banks::{load_questions, parse_qtype};
use crate::error::AppError;
use crate::AppState;
use axum::http::StatusCode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotItem {
    pub question_id: String,
    pub qtype: QType,
    pub stem: String,
    pub stem_html: String,
    pub options: Vec<OptionItem>,
    pub option_map: Vec<(String, String)>,
    pub points: u32,
    pub answer: Answer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum StudentAnswer {
    Single(String),
    Multiple(Vec<String>),
    Judge(bool),
    Fill(String),
    Code(String),
}

impl StudentAnswer {
    pub fn to_response(&self) -> StudentResponse {
        match self {
            StudentAnswer::Single(v) => StudentResponse::Single(v.clone()),
            StudentAnswer::Multiple(v) => StudentResponse::Multiple(v.clone()),
            StudentAnswer::Judge(v) => StudentResponse::Judge(*v),
            StudentAnswer::Fill(v) => StudentResponse::Fill(v.clone()),
            StudentAnswer::Code(v) => StudentResponse::Code(v.clone()),
        }
    }

    pub fn exam_keys(&self) -> Vec<String> {
        match self {
            StudentAnswer::Single(v) => vec![v.clone()],
            StudentAnswer::Multiple(v) => v.clone(),
            _ => vec![],
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnswersBlob {
    #[serde(default)]
    pub items: HashMap<String, StudentAnswer>,
    #[serde(default)]
    pub code_scores: HashMap<String, u32>,
    #[serde(default)]
    pub item_scores: HashMap<String, Option<u32>>,
    #[serde(default)]
    pub item_correct: HashMap<String, Option<bool>>,
}

#[derive(Debug, Clone)]
pub struct TaskRow {
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
}

#[derive(Debug, Clone)]
pub struct RuleRow {
    pub bank_ids: Vec<String>,
    pub type_counts: HashMap<QType, usize>,
    pub shuffle_within_section: bool,
    pub shuffle_options: bool,
    pub type_subtotals: Option<HashMap<QType, u32>>,
    pub score_plan: Option<HashMap<QType, Vec<u32>>>,
}

pub fn parse_qtype_map<T: Clone>(raw: &HashMap<String, T>) -> HashMap<QType, T> {
    let mut out = HashMap::new();
    for (k, v) in raw {
        out.insert(parse_qtype(k), v.clone());
    }
    out
}

pub fn qtype_map_to_json<T: Serialize>(m: &HashMap<QType, T>) -> Value {
    let mut map = serde_json::Map::new();
    for q in TYPE_ORDER {
        if let Some(v) = m.get(&q) {
            map.insert(
                crate::banks::qtype_str(q).to_string(),
                serde_json::to_value(v).unwrap_or(Value::Null),
            );
        }
    }
    Value::Object(map)
}

pub async fn load_task(state: &AppState, id: &str) -> Result<TaskRow, AppError> {
    let row = sqlx::query(
        r#"SELECT id, title, mode, status, duration_minutes, allow_early_submit, total_score, pass_score, published_at, closes_at
           FROM exam_tasks WHERE id = ?"#,
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "考试任务不存在"))?;
    Ok(task_from_row(row))
}

pub fn task_from_row(row: sqlx::sqlite::SqliteRow) -> TaskRow {
    TaskRow {
        id: row.get("id"),
        title: row.get("title"),
        mode: row.get("mode"),
        status: row.get("status"),
        duration_minutes: row.get("duration_minutes"),
        allow_early_submit: row.get::<i64, _>("allow_early_submit") != 0,
        total_score: row.get("total_score"),
        pass_score: row.get("pass_score"),
        published_at: row.get("published_at"),
        closes_at: row.get("closes_at"),
    }
}

pub async fn load_rule(state: &AppState, task_id: &str) -> Result<RuleRow, AppError> {
    let row = sqlx::query(
        r#"SELECT bank_ids_json, type_counts_json, shuffle_within_section, shuffle_options, type_subtotals_json, score_plan_json
           FROM paper_rules WHERE task_id = ?"#,
    )
    .bind(task_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "组卷规则不存在"))?;
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
    Ok(RuleRow {
        bank_ids,
        type_counts: parse_qtype_map(&counts),
        shuffle_within_section: row.get::<i64, _>("shuffle_within_section") != 0,
        shuffle_options: row.get::<i64, _>("shuffle_options") != 0,
        type_subtotals: subtotals.as_ref().map(parse_qtype_map),
        score_plan: plan.as_ref().map(parse_qtype_map),
    })
}

pub async fn load_bank_questions(
    state: &AppState,
    bank_ids: &[String],
) -> Result<Vec<(String, Question)>, AppError> {
    let mut out = Vec::new();
    for bid in bank_ids {
        for q in load_questions(state, bid).await? {
            out.push((
                q.id,
                Question {
                    qtype: q.qtype,
                    stem: q.stem,
                    stem_html: q.stem_html,
                    options: q.options,
                    answer: q.answer,
                    source: q.source,
                    paper_title: q.paper_title,
                },
            ));
        }
    }
    Ok(out)
}

pub fn to_paper_rule(rule: &RuleRow, total_score: u32) -> PaperRule {
    PaperRule {
        type_counts: rule.type_counts.clone(),
        shuffle_within_section: rule.shuffle_within_section,
        shuffle_options: rule.shuffle_options,
        total_score,
        type_subtotals: rule.type_subtotals.clone(),
    }
}

pub fn compute_score_plan(rule: &RuleRow, total_score: u32) -> Result<HashMap<QType, Vec<u32>>, AppError> {
    let mut picked = Vec::new();
    for qtype in TYPE_ORDER {
        let n = rule.type_counts.get(&qtype).copied().unwrap_or(0);
        for _ in 0..n {
            picked.push((qtype, 0usize));
        }
    }
    let plan = allocate_scores(&to_paper_rule(rule, total_score), &picked).map_err(gen_err)?;
    let mut grouped: HashMap<QType, Vec<u32>> = HashMap::new();
    let mut i = 0;
    for qtype in TYPE_ORDER {
        let n = rule.type_counts.get(&qtype).copied().unwrap_or(0);
        if n == 0 {
            continue;
        }
        grouped.insert(qtype, plan[i..i + n].to_vec());
        i += n;
    }
    Ok(grouped)
}

pub fn check_pool(pool: &[(String, Question)], rule: &RuleRow) -> Result<(), AppError> {
    for qtype in TYPE_ORDER {
        let need = rule.type_counts.get(&qtype).copied().unwrap_or(0);
        if need == 0 {
            continue;
        }
        let available = pool.iter().filter(|(_, q)| q.qtype == qtype).count();
        if available < need {
            return Err(AppError::new(
                StatusCode::CONFLICT,
                "NOT_ENOUGH",
                format!("{qtype:?} 现有 {available} 题，规则要求 {need} 题"),
            )
            .with_details(json!({
                "qtype": crate::banks::qtype_str(qtype),
                "available": available,
                "required": need,
            })));
        }
    }
    Ok(())
}

pub fn generate_snapshot(
    pool: &[(String, Question)],
    rule: &RuleRow,
    total_score: u32,
) -> Result<Vec<SnapshotItem>, AppError> {
    check_pool(pool, rule)?;
    let questions: Vec<Question> = pool.iter().map(|(_, q)| q.clone()).collect();
    let snap = generate(
        &questions,
        &to_paper_rule(rule, total_score),
        &mut StdRng::from_entropy(),
    )
    .map_err(gen_err)?;
    let mut items = Vec::new();
    for it in snap.items {
        let (id, q) = &pool[it.source_index];
        items.push(SnapshotItem {
            question_id: id.clone(),
            qtype: it.qtype,
            stem: q.stem.clone(),
            stem_html: q.stem_html.clone(),
            options: it.options,
            option_map: it.option_map,
            points: it.points,
            answer: q.answer.clone(),
        });
    }
    Ok(items)
}

pub fn student_items(items: &[SnapshotItem], exam_hide_points: bool) -> Vec<Value> {
    items
        .iter()
        .map(|it| {
            let mut v = json!({
                "question_id": it.question_id,
                "qtype": it.qtype,
                "stem": it.stem,
                "stem_html": it.stem_html,
                "options": it.options,
            });
            if !exam_hide_points {
                v["points"] = json!(it.points);
            }
            v
        })
        .collect()
}

pub fn parse_items(raw: &str) -> Vec<SnapshotItem> {
    serde_json::from_str(raw).unwrap_or_default()
}

pub fn parse_answers(raw: &str) -> AnswersBlob {
    serde_json::from_str(raw).unwrap_or_default()
}

pub fn grade_blob(items: &[SnapshotItem], blob: &mut AnswersBlob) -> (Option<i64>, Option<i64>, bool) {
    let mut objective: i64 = 0;
    let mut pending_code = false;
    let mut total: i64 = 0;
    let mut total_known = true;
    for it in items {
        if it.qtype == QType::Code {
            pending_code = blob.code_scores.get(&it.question_id).is_none();
            if let Some(&s) = blob.code_scores.get(&it.question_id) {
                blob.item_scores
                    .insert(it.question_id.clone(), Some(s));
                blob.item_correct
                    .insert(it.question_id.clone(), Some(s == it.points));
                total += s as i64;
            } else {
                blob.item_scores.insert(it.question_id.clone(), None);
                blob.item_correct.insert(it.question_id.clone(), None);
                total_known = false;
            }
            continue;
        }
        let resp = blob.items.get(&it.question_id);
        let outcome = if let Some(ans) = resp {
            let restored = restore_original_keys(&ans.exam_keys(), &it.option_map);
            grade(
                it.qtype,
                &it.answer,
                &ans.to_response(),
                it.points,
                Some(&restored),
            )
        } else {
            grading::GradeOutcome {
                awarded: Some(0),
                correct: Some(false),
            }
        };
        blob.item_scores
            .insert(it.question_id.clone(), outcome.awarded);
        blob.item_correct
            .insert(it.question_id.clone(), outcome.correct);
        if let Some(s) = outcome.awarded {
            objective += s as i64;
            total += s as i64;
        }
    }
    (
        Some(objective),
        if total_known { Some(total) } else { Some(objective) },
        pending_code,
    )
}

pub fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

pub fn task_expired(task: &TaskRow) -> bool {
    if task.status != "published" {
        return false;
    }
    task.closes_at
        .as_deref()
        .and_then(parse_rfc3339)
        .map(|t| Utc::now() >= t)
        .unwrap_or(false)
}

fn gen_err(e: GenError) -> AppError {
    match e {
        GenError::NotEnough {
            qtype,
            available,
            required,
        } => AppError::new(
            StatusCode::CONFLICT,
            "NOT_ENOUGH",
            format!("{qtype:?} 现有 {available} 题，规则要求 {required} 题"),
        )
        .with_details(json!({
            "qtype": crate::banks::qtype_str(qtype),
            "available": available,
            "required": required,
        })),
        GenError::TotalLessThanCount { total, count } => AppError::new(
            StatusCode::CONFLICT,
            "TOTAL_TOO_SMALL",
            format!("总分 {total} 必须大于等于题量 {count}"),
        )
        .with_details(json!({"total": total, "count": count})),
    }
}
