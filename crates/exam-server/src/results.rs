use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashMap;

use crate::error::AppError;
use crate::paper::{grade_blob, load_task, parse_answers, parse_items};
use crate::AppState;

#[derive(Deserialize)]
pub struct CodeScoresBody {
    pub scores: std::collections::HashMap<String, u32>,
}

#[derive(Deserialize)]
pub struct AnalysisQuery {
    pub pass_score: Option<i64>,
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
    for (qid, score) in &body.scores {
        let item = items.iter().find(|i| &i.question_id == qid).ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "BAD_REQUEST", "题目不在该试卷中")
        })?;
        if item.qtype != bank_html::QType::Code {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "只能给编程题打分",
            ));
        }
        if *score > item.points {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                format!("分数不能超过该题满分 {}", item.points),
            ));
        }
        blob.code_scores.insert(qid.clone(), *score);
    }
    let (obj, total, _) = grade_blob(&items, &mut blob);
    sqlx::query("UPDATE attempts SET answers_json=?, objective_score=?, total_score=? WHERE id=?")
        .bind(serde_json::to_string(&blob).unwrap_or_else(|_| "{}".into()))
        .bind(obj)
        .bind(total)
        .bind(&id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"objective_score": obj, "total_score": total})))
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

pub async fn analysis(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<AnalysisQuery>,
) -> Result<Json<Value>, AppError> {
    let task = load_task(&state, &id).await?;
    let pass = q.pass_score.unwrap_or(task.pass_score);
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
    let excellent_cut = (total * 0.9).ceil() as i64;
    let good_cut = (total * 0.8).ceil() as i64;
    let excellent = scores.iter().filter(|s| **s >= excellent_cut).count();
    let good = scores.iter().filter(|s| **s >= good_cut).count();
    let full_mark = scores.iter().filter(|s| **s >= task.total_score).count();
    let failed = scores.iter().filter(|s| **s < pass).count();
    let mut bands = [0u32; 5];
    for s in &scores {
        let p = *s as f64 / total;
        let idx = if p < 0.6 {
            0
        } else if p < 0.7 {
            1
        } else if p < 0.8 {
            2
        } else if p < 0.9 {
            3
        } else {
            4
        };
        bands[idx] += 1;
    }
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
    Ok(Json(json!({
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
        "pass_rate": if n == 0 { 0.0 } else { passed as f64 / n as f64 },
        "fail_rate": if n == 0 { 0.0 } else { failed as f64 / n as f64 },
        "excellent_rate": if n == 0 { 0.0 } else { excellent as f64 / n as f64 },
        "good_rate": if n == 0 { 0.0 } else { good as f64 / n as f64 },
        "questions": per_question,
        "type_rates": type_rates,
        "classes": classes,
        "bands": {
            "below_60": bands[0],
            "60_70": bands[1],
            "70_80": bands[2],
            "80_90": bands[3],
            "90_100": bands[4],
        }
    })))
}

async fn load_result_rows(state: &AppState, task_id: &str) -> Result<Vec<Value>, AppError> {
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
        grade_blob(&items, &mut blob);
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
            "code_items": items
                .iter()
                .filter(|it| it.qtype == bank_html::QType::Code)
                .map(|it| {
                    json!({
                        "question_id": it.question_id,
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
