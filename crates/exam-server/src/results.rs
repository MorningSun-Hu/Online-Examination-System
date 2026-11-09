use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashMap;

use crate::error::AppError;
use crate::paper::{exam_facing_answer, grade_blob, load_task, needs_manual, parse_answers, parse_items};
use crate::AppState;

#[derive(Deserialize)]
pub struct CodeScoresBody {
    pub scores: std::collections::HashMap<String, u32>,
}

#[derive(Deserialize)]
pub struct AnalysisQuery {
    pub pass_score: Option<i64>,
    pub excellent_score: Option<i64>,
    pub good_score: Option<i64>,
    pub bands: Option<String>,
    pub top_pct: Option<f64>,
    pub bottom_pct: Option<f64>,
}

pub async fn code_scores(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CodeScoresBody>,
) -> Result<Json<Value>, AppError> {
    let row = sqlx::query(
        "SELECT a.id, a.answers_json, p.items_json, p.task_id FROM attempts a JOIN papers p ON p.id=a.paper_id WHERE a.id=?",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "答卷不存在"))?;
    let items = parse_items(&row.get::<String, _>("items_json"));
    let mut blob = parse_answers(&row.get::<String, _>("answers_json"));
    let mut changed = false;
    let task = load_task(&state, &row.get::<String, _>("task_id")).await?;
    if task.status == "released" {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "BAD_STATE",
            "试卷已公开，不能再改批改结果",
        ));
    }
    for (qid, score) in &body.scores {
        let item = items.iter().find(|i| &i.question_id == qid).ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "BAD_REQUEST", "题目不在该试卷中")
        })?;
        if !needs_manual(item, task.auto_grade) {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "该题当前为自动批改，不能手工给分",
            ));
        }
        if *score > item.points {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                format!("分数不能超过该题满分 {}", item.points),
            ));
        }
        if blob.code_scores.get(qid) != Some(score) {
            changed = true;
        }
        blob.code_scores.insert(qid.clone(), *score);
    }
    if changed {
        blob.marking_confirmed = false;
    }
    let (obj, total, _) = grade_blob(&items, &mut blob, task.auto_grade);
    sqlx::query("UPDATE attempts SET answers_json=?, objective_score=?, total_score=? WHERE id=?")
        .bind(serde_json::to_string(&blob).unwrap_or_else(|_| "{}".into()))
        .bind(obj)
        .bind(total)
        .bind(&id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"objective_score": obj, "total_score": total})))
}

pub async fn confirm_marking(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let row = sqlx::query(
        "SELECT a.id, a.status, a.answers_json, p.items_json, p.task_id FROM attempts a JOIN papers p ON p.id=a.paper_id WHERE a.id=?",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "答卷不存在"))?;
    let attempt_status: String = row.get("status");
    if attempt_status != "submitted" {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "NOT_SUBMITTED",
            "该学生尚未交卷",
        ));
    }
    let task = load_task(&state, &row.get::<String, _>("task_id")).await?;
    if task.status == "released" {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "BAD_STATE",
            "试卷已公开，不能再改批改结果",
        ));
    }
    let items = parse_items(&row.get::<String, _>("items_json"));
    let mut blob = parse_answers(&row.get::<String, _>("answers_json"));
    let manuals: Vec<_> = items
        .iter()
        .filter(|it| needs_manual(it, task.auto_grade))
        .collect();
    if manuals.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "该生没有需要确认的题目",
        ));
    }
    if manuals
        .iter()
        .any(|it| !blob.code_scores.contains_key(&it.question_id))
    {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "PENDING_CODE",
            "请先给完该生全部待批改题目",
        ));
    }
    blob.marking_confirmed = true;
    let (obj, total, _) = grade_blob(&items, &mut blob, task.auto_grade);
    sqlx::query("UPDATE attempts SET answers_json=?, objective_score=?, total_score=? WHERE id=?")
        .bind(serde_json::to_string(&blob).unwrap_or_else(|_| "{}".into()))
        .bind(obj)
        .bind(total)
        .bind(&id)
        .execute(&state.pool)
        .await?;
    crate::tasks::maybe_mark_graded(&state, &row.get::<String, _>("task_id")).await?;
    Ok(Json(json!({
        "marking_confirmed": true,
        "objective_score": obj,
        "total_score": total
    })))
}

pub async fn list_results(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let rows = load_result_rows(&state, &id).await?;
    Ok(Json(json!({ "results": rows })))
}

pub async fn export_csv(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(HeaderMap, Vec<u8>), AppError> {
    let rows = load_result_rows(&state, &id).await?;
    let mut max_q = 0usize;
    for r in &rows {
        max_q = max_q.max(r["item_scores"].as_array().map(|a| a.len()).unwrap_or(0));
    }
    let mut csv = String::from("\u{feff}学号,姓名,班级,总分");
    for i in 1..=max_q {
        csv.push_str(&format!(",第{i}题"));
    }
    csv.push_str(",提交时间\n");
    for r in rows {
        csv.push_str(&escape_csv(r["student_no"].as_str().unwrap_or("")));
        csv.push(',');
        csv.push_str(&escape_csv(r["name"].as_str().unwrap_or("")));
        csv.push(',');
        csv.push_str(&escape_csv(r["class_name"].as_str().unwrap_or("")));
        csv.push(',');
        csv.push_str(&r["total_score"].to_string().replace("null", ""));
        let scores = r["item_scores"].as_array().cloned().unwrap_or_default();
        for i in 0..max_q {
            csv.push(',');
            if let Some(v) = scores.get(i) {
                if !v.is_null() {
                    csv.push_str(&v.to_string());
                }
            }
        }
        csv.push(',');
        csv.push_str(&escape_csv(r["submitted_at"].as_str().unwrap_or("")));
        csv.push('\n');
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/csv; charset=utf-8".parse().unwrap(),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        "attachment; filename=\"results.csv\"".parse().unwrap(),
    );
    Ok((headers, csv.into_bytes()))
}

fn percentile(sorted: &[i64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let x = (sorted.len() as f64 - 1.0) * p;
    let i = x.floor() as usize;
    let f = x - i as f64;
    let a = sorted[i] as f64;
    let b = sorted[(i + 1).min(sorted.len() - 1)] as f64;
    a + (b - a) * f
}

fn stddev_sample(scores: &[i64], avg: f64) -> f64 {
    let n = scores.len();
    if n < 2 {
        return 0.0;
    }
    let var = scores
        .iter()
        .map(|s| {
            let d = *s as f64 - avg;
            d * d
        })
        .sum::<f64>()
        / (n as f64 - 1.0);
    var.sqrt()
}

fn pct_cut(total: i64, p: f64) -> i64 {
    ((total.max(1) as f64) * p).ceil() as i64
}

fn parse_band_cuts(raw: Option<&str>, total: i64) -> [i64; 5] {
    let mut cuts = [
        pct_cut(total, 0.6),
        pct_cut(total, 0.7),
        pct_cut(total, 0.8),
        pct_cut(total, 0.9),
        pct_cut(total, 0.95),
    ];
    if let Some(s) = raw.map(str::trim).filter(|x| !x.is_empty()) {
        let parts: Vec<i64> = s
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        if parts.len() >= 5 {
            cuts = [parts[0], parts[1], parts[2], parts[3], parts[4]];
        } else if parts.len() == 4 {
            cuts = [parts[0], parts[1], parts[2], parts[3], pct_cut(total, 0.95)];
        }
    }
    cuts.sort();
    for i in 1..5 {
        if cuts[i] <= cuts[i - 1] {
            cuts[i] = cuts[i - 1] + 1;
        }
    }
    cuts
}

fn band_index(score: i64, cuts: &[i64; 5]) -> usize {
    for (i, cut) in cuts.iter().enumerate() {
        if score < *cut {
            return i;
        }
    }
    cuts.len()
}

fn band_labels(cuts: &[i64; 5]) -> [String; 6] {
    [
        format!("<{}", cuts[0]),
        format!("{}-{}", cuts[0], cuts[1].saturating_sub(1)),
        format!("{}-{}", cuts[1], cuts[2].saturating_sub(1)),
        format!("{}-{}", cuts[2], cuts[3].saturating_sub(1)),
        format!("{}-{}", cuts[3], cuts[4].saturating_sub(1)),
        format!("≥{}", cuts[4]),
    ]
}

fn default_tail_pct(n: usize) -> f64 {
    if n == 0 {
        10.0
    } else {
        (10.0 / n as f64 * 100.0).clamp(1.0, 100.0)
    }
}

fn people_json(row: &(i64, String, String, String, String)) -> Value {
    json!({
        "total_score": row.0,
        "student_no": row.1,
        "name": row.2,
        "class_name": row.3,
        "attempt_id": row.4,
    })
}

async fn task_live_draw(state: &AppState, task: &crate::paper::TaskRow) -> Result<bool, AppError> {
    let Some(pid) = task.paper_id.as_deref() else {
        return Ok(true);
    };
    let v: Option<i64> = sqlx::query_scalar("SELECT live_draw FROM exam_papers WHERE id = ?")
        .bind(pid)
        .fetch_optional(&state.pool)
        .await?;
    Ok(v.map(|x| x != 0).unwrap_or(true))
}

pub async fn analysis(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<AnalysisQuery>,
) -> Result<Json<Value>, AppError> {
    let task = load_task(&state, &id).await?;
    let pass = q.pass_score.unwrap_or(task.pass_score);
    let mut good_cut = q.good_score.unwrap_or(pct_cut(task.total_score, 0.8));
    let mut excellent_cut = q.excellent_score.unwrap_or(pct_cut(task.total_score, 0.9));
    if good_cut < pass {
        good_cut = pass;
    }
    if excellent_cut < good_cut {
        excellent_cut = good_cut;
    }
    let cuts = parse_band_cuts(q.bands.as_deref(), task.total_score);
    let rows = load_result_rows(&state, &id).await?;
    let roster_n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_roster WHERE task_id = ?")
        .bind(&id)
        .fetch_one(&state.pool)
        .await?;
    let catalog_rows = sqlx::query(
        r#"SELECT p.items_json FROM papers p
           JOIN attempts a ON a.paper_id = p.id
           WHERE p.task_id = ? AND a.status = 'submitted'"#,
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await?;
    let mut catalog: HashMap<String, crate::paper::SnapshotItem> = HashMap::new();
    for row in &catalog_rows {
        for it in parse_items(&row.get::<String, _>("items_json")) {
            catalog.entry(it.question_id.clone()).or_insert(it);
        }
    }
    let mut scores: Vec<i64> = rows
        .iter()
        .filter_map(|r| r["total_score"].as_i64())
        .collect();
    scores.sort();
    let n = scores.len() as i64;
    let avg = if n == 0 {
        0.0
    } else {
        scores.iter().sum::<i64>() as f64 / n as f64
    };
    let max = scores.last().copied().unwrap_or(0);
    let min = scores.first().copied().unwrap_or(0);
    let median = percentile(&scores, 0.5);
    let q1 = percentile(&scores, 0.25);
    let q3 = percentile(&scores, 0.75);
    let sd = stddev_sample(&scores, avg);
    let passed = scores.iter().filter(|s| **s >= pass).count();
    let total = task.total_score.max(1) as f64;
    let excellent = scores.iter().filter(|s| **s >= excellent_cut).count();
    let good = scores.iter().filter(|s| **s >= good_cut).count();
    let full_mark = scores.iter().filter(|s| **s >= task.total_score).count();
    let failed = scores.iter().filter(|s| **s < pass).count();
    let mut bands = [0u32; 6];
    for s in &scores {
        bands[band_index(*s, &cuts)] += 1;
    }
    let good_only = scores
        .iter()
        .filter(|s| **s >= good_cut && **s < excellent_cut)
        .count();
    let pass_only = scores
        .iter()
        .filter(|s| **s >= pass && **s < good_cut)
        .count();
    let plus_cut = cuts[4].max(excellent_cut);
    let excellent_plus = scores.iter().filter(|s| **s >= plus_cut).count();
    let excellent_only = scores
        .iter()
        .filter(|s| **s >= excellent_cut && **s < plus_cut)
        .count();
    let mut per_question: Vec<Value> = Vec::new();
    let mut type_got: HashMap<String, (f64, f64, f64, f64, usize)> = HashMap::new();
    if let Some(first) = rows.first() {
        let qcount = first["item_scores"].as_array().map(|a| a.len()).unwrap_or(0);
        for i in 0..qcount {
            let mut correct = 0.0;
            let mut got = 0.0;
            let mut pts = 0.0;
            let mut qtype = String::new();
            let mut nq = 0.0;
            for r in &rows {
                let scores = r["item_scores"].as_array().cloned().unwrap_or_default();
                let types = r["item_types"].as_array().cloned().unwrap_or_default();
                let points = r["item_points"].as_array().cloned().unwrap_or_default();
                let corrects = r["item_correct"].as_array().cloned().unwrap_or_default();
                if let Some(t) = types.get(i).and_then(|v| v.as_str()) {
                    qtype = t.to_string();
                }
                if let Some(p) = points.get(i).and_then(|v| v.as_f64()).or_else(|| points.get(i).and_then(|v| v.as_i64()).map(|x| x as f64)) {
                    pts += p;
                }
                if let Some(s) = scores.get(i).and_then(|v| v.as_f64()).or_else(|| scores.get(i).and_then(|v| v.as_i64()).map(|x| x as f64)) {
                    got += s;
                }
                if corrects.get(i).and_then(|v| v.as_bool()) == Some(true) {
                    correct += 1.0;
                }
                nq += 1.0;
            }
            let rate = if nq == 0.0 { 0.0 } else { correct / nq };
            let score_rate = if pts == 0.0 { 0.0 } else { got / pts };
            let qid = first["item_ids"]
                .as_array()
                .and_then(|a| a.get(i))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let item = catalog.get(&qid);
            per_question.push(json!({
                "index": i + 1,
                "question_id": qid,
                "qtype": qtype,
                "correct_rate": rate,
                "score_rate": score_rate,
                "avg_score": if nq == 0.0 { 0.0 } else { got / nq },
                "points": item.map(|it| it.points).unwrap_or(0),
                "n": nq,
                "stem": item.map(|it| it.stem.clone()).unwrap_or_default(),
                "stem_html": item.map(|it| it.stem_html.clone()).unwrap_or_default(),
                "options": item.map(|it| it.options.clone()).unwrap_or_default(),
                "answer": item.map(|it| it.answer.clone()),
            }));
            let e = type_got.entry(qtype).or_insert((0.0, 0.0, 0.0, 0.0, 0));
            e.0 += got;
            e.1 += pts;
            e.2 += correct;
            e.3 += nq;
            e.4 += 1;
        }
    }
    let live_draw = task_live_draw(&state, &task).await?;
    let question_count = per_question.len();
    if live_draw {
        per_question.clear();
    }
    let type_rates: Value = type_got
        .into_iter()
        .map(|(k, (g, p, c, nn, cnt))| {
            json!({
                "qtype": k,
                "count": cnt,
                "score_rate": if p == 0.0 { 0.0 } else { g / p },
                "correct_rate": if nn == 0.0 { 0.0 } else { c / nn },
            })
        })
        .collect();
    let mut class_map: HashMap<String, Vec<i64>> = HashMap::new();
    for r in &rows {
        let cn = r["class_name"].as_str().unwrap_or("").to_string();
        if let Some(s) = r["total_score"].as_i64() {
            class_map.entry(cn).or_default().push(s);
        }
    }
    let mut classes: Vec<Value> = class_map
        .into_iter()
        .map(|(name, mut ss)| {
            ss.sort();
            let cn = ss.len() as i64;
            let cavg = if cn == 0 {
                0.0
            } else {
                ss.iter().sum::<i64>() as f64 / cn as f64
            };
            let cpass = ss.iter().filter(|s| **s >= pass).count();
            json!({
                "class_name": name,
                "count": cn,
                "average": cavg,
                "max": ss.last().copied().unwrap_or(0),
                "min": ss.first().copied().unwrap_or(0),
                "pass_rate": if cn == 0 { 0.0 } else { cpass as f64 / cn as f64 },
            })
        })
        .collect();
    classes.sort_by(|a, b| {
        a["class_name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["class_name"].as_str().unwrap_or(""))
    });
    let mut people: Vec<(i64, String, String, String, String)> = Vec::new();
    for r in &rows {
        if let Some(s) = r["total_score"].as_i64() {
            people.push((
                s,
                r["student_no"].as_str().unwrap_or("").to_string(),
                r["name"].as_str().unwrap_or("").to_string(),
                r["class_name"].as_str().unwrap_or("").to_string(),
                r["attempt_id"].as_str().unwrap_or("").to_string(),
            ));
        }
    }
    people.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let pn = people.len();
    let default_pct = default_tail_pct(pn);
    let top_pct = q.top_pct.unwrap_or(default_pct).clamp(1.0, 100.0);
    let bottom_pct = q.bottom_pct.unwrap_or(default_pct).clamp(1.0, 100.0);
    let top_n = if pn == 0 {
        0
    } else {
        ((top_pct / 100.0) * pn as f64).ceil() as usize
    }
    .min(pn)
    .max(if pn == 0 { 0 } else { 1 });
    let bottom_n = if pn == 0 {
        0
    } else {
        ((bottom_pct / 100.0) * pn as f64).ceil() as usize
    }
    .min(pn)
    .max(if pn == 0 { 0 } else { 1 });
    let top_students: Vec<Value> = people.iter().take(top_n).map(people_json).collect();
    let bottom_students: Vec<Value> = people.iter().rev().take(bottom_n).map(people_json).collect();
    let labels = band_labels(&cuts);
    let rate = |c: usize| if n == 0 { 0.0 } else { c as f64 / n as f64 };
    let cv = if avg.abs() < f64::EPSILON { 0.0 } else { sd / avg };
    let submit_rate = if roster_n == 0 {
        0.0
    } else {
        n as f64 / roster_n as f64
    };
    let mut body = json!({
        "count": n,
        "roster_count": roster_n,
        "average": avg,
        "avg_rate": avg / total,
        "max": max,
        "min": min,
        "median": median,
        "q1": q1,
        "q3": q3,
        "stddev": sd,
        "pass_score": pass,
        "excellent_score": excellent_cut,
        "total_score": task.total_score,
        "passed": passed,
        "failed": failed,
        "excellent": excellent,
        "good": good,
        "full_mark": full_mark,
        "pass_rate": rate(passed),
        "fail_rate": rate(failed),
        "excellent_rate": rate(excellent),
        "good_rate": rate(good),
        "questions": per_question,
        "live_draw": live_draw,
        "question_count": question_count,
        "type_rates": type_rates,
        "classes": classes,
        "bands": {
            "cuts": cuts,
            "labels": labels,
            "counts": bands,
            "below_60": bands[0],
            "60_70": bands[1],
            "70_80": bands[2],
            "80_90": bands[3],
            "90_95": bands[4],
            "95_100": bands[5],
        }
    });
    if let Value::Object(obj) = &mut body {
        obj.insert("good_score".into(), json!(good_cut));
        obj.insert("full_rate".into(), json!(rate(full_mark)));
        obj.insert("submit_rate".into(), json!(submit_rate));
        obj.insert("cv".into(), json!(cv));
        obj.insert("excellent_only".into(), json!(excellent_only));
        obj.insert("good_only".into(), json!(good_only));
        obj.insert("pass_only".into(), json!(pass_only));
        obj.insert("excellent_only_rate".into(), json!(rate(excellent_only)));
        obj.insert("good_only_rate".into(), json!(rate(good_only)));
        obj.insert("pass_only_rate".into(), json!(rate(pass_only)));
        obj.insert("excellent_plus".into(), json!(excellent_plus));
        obj.insert("excellent_plus_score".into(), json!(plus_cut));
        obj.insert("excellent_plus_rate".into(), json!(rate(excellent_plus)));
        obj.insert("top_pct".into(), json!(top_pct));
        obj.insert("bottom_pct".into(), json!(bottom_pct));
        obj.insert("top_n".into(), json!(top_n));
        obj.insert("bottom_n".into(), json!(bottom_n));
        obj.insert("top_students".into(), json!(top_students));
        obj.insert("bottom_students".into(), json!(bottom_students));
    }
    Ok(Json(body))
}

pub async fn attempt_paper(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let row = sqlx::query(
        r#"SELECT a.id, a.status, a.answers_json, a.objective_score, a.total_score, a.submitted_at,
                  p.items_json, p.task_id, s.student_no, s.name, s.class_name
           FROM attempts a
           JOIN papers p ON p.id = a.paper_id
           JOIN students s ON s.id = p.student_id
           WHERE a.id = ?"#,
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "答卷不存在"))?;
    let status: String = row.get("status");
    if status != "submitted" {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "NOT_SUBMITTED",
            "该学生尚未交卷",
        ));
    }
    let task_id: String = row.get("task_id");
    let task = load_task(&state, &task_id).await?;
    let items = parse_items(&row.get::<String, _>("items_json"));
    let mut blob = parse_answers(&row.get::<String, _>("answers_json"));
    grade_blob(&items, &mut blob, task.auto_grade);
    let details: Vec<Value> = items
        .iter()
        .map(|it| {
            json!({
                "question_id": it.question_id,
                "qtype": crate::banks::qtype_str(it.qtype),
                "stem": it.stem,
                "stem_html": it.stem_html,
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
        "attempt_id": row.get::<String, _>("id"),
        "task_id": task_id,
        "title": task.title,
        "student_no": row.get::<String, _>("student_no"),
        "name": row.get::<String, _>("name"),
        "class_name": row.get::<String, _>("class_name"),
        "submitted_at": row.get::<Option<String>, _>("submitted_at"),
        "objective_score": row.get::<Option<i64>, _>("objective_score"),
        "total_score": row.get::<Option<i64>, _>("total_score"),
        "paper_total": task.total_score,
        "items": details,
    })))
}

async fn load_result_rows(state: &AppState, task_id: &str) -> Result<Vec<Value>, AppError> {
    let task = load_task(state, task_id).await?;
    let rows = sqlx::query(
        r#"SELECT s.student_no, s.name, s.class_name, a.client_id, a.total_score, a.submitted_at,
                  a.answers_json, p.items_json, a.id as attempt_id
           FROM papers p
           JOIN attempts a ON a.paper_id = p.id
           JOIN students s ON s.id = p.student_id
           WHERE p.task_id = ? AND a.status = 'submitted'
           ORDER BY s.class_name, s.student_no, p.attempt_no"#,
    )
    .bind(task_id)
    .fetch_all(&state.pool)
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let items = parse_items(&row.get::<String, _>("items_json"));
        let mut blob = parse_answers(&row.get::<String, _>("answers_json"));
        grade_blob(&items, &mut blob, task.auto_grade);
        let item_scores: Vec<Value> = items
            .iter()
            .map(|it| match blob.item_scores.get(&it.question_id) {
                Some(Some(s)) => json!(s),
                _ => Value::Null,
            })
            .collect();
        let item_correct: Vec<Value> = items
            .iter()
            .map(|it| match blob.item_correct.get(&it.question_id) {
                Some(Some(s)) => json!(s),
                _ => Value::Null,
            })
            .collect();
        let item_types: Vec<Value> = items
            .iter()
            .map(|it| json!(crate::banks::qtype_str(it.qtype)))
            .collect();
        let item_points: Vec<Value> = items.iter().map(|it| json!(it.points)).collect();
        out.push(json!({
            "attempt_id": row.get::<String, _>("attempt_id"),
            "student_no": row.get::<String, _>("student_no"),
            "name": row.get::<String, _>("name"),
            "class_name": row.get::<String, _>("class_name"),
            "client_id": row.get::<String, _>("client_id"),
            "total_score": row.get::<Option<i64>, _>("total_score"),
            "submitted_at": row.get::<Option<String>, _>("submitted_at"),
            "item_scores": item_scores,
            "item_correct": item_correct,
            "item_types": item_types,
            "item_points": item_points,
            "item_ids": items.iter().map(|it| json!(it.question_id)).collect::<Vec<_>>(),
            "item_stems": items.iter().map(|it| json!(it.stem)).collect::<Vec<_>>(),
            "marking_confirmed": blob.marking_confirmed,
            "code_items": items
                .iter()
                .filter(|it| needs_manual(it, task.auto_grade))
                .map(|it| {
                    json!({
                        "question_id": it.question_id,
                        "qtype": crate::banks::qtype_str(it.qtype),
                        "stem": it.stem,
                        "points": it.points,
                        "response": blob.items.get(&it.question_id),
                        "score": blob.code_scores.get(&it.question_id),
                    })
                })
                .collect::<Vec<_>>(),
        }));
    }
    Ok(out)
}

fn escape_csv(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
