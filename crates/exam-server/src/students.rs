use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use ulid::Ulid;

use crate::error::AppError;
use crate::AppState;

#[derive(Deserialize)]
pub struct ImportCsvBody {
    #[serde(default = "default_filename")]
    pub filename: String,
    pub csv: String,
}

fn default_filename() -> String {
    "students.csv".into()
}

#[derive(Serialize)]
pub struct CsvFailure {
    pub line: usize,
    pub reason: String,
}

#[derive(Serialize)]
pub struct ImportCsvResult {
    pub filename: String,
    pub upserted: usize,
    pub failed: usize,
    pub failures: Vec<CsvFailure>,
}

#[derive(Serialize)]
pub struct StudentRow {
    pub id: String,
    pub student_no: String,
    pub name: String,
    pub class_name: String,
}

#[derive(Serialize)]
pub struct StudentList {
    pub students: Vec<StudentRow>,
}

#[derive(Deserialize)]
pub struct RosterBody {
    pub student_ids: Vec<String>,
}

#[derive(Serialize)]
pub struct RosterList {
    pub student_ids: Vec<String>,
    pub students: Vec<StudentRow>,
}

pub async fn list_students(
    State(state): State<AppState>,
) -> Result<Json<StudentList>, AppError> {
    let rows = sqlx::query(
        "SELECT id, student_no, name, class_name FROM students ORDER BY class_name, student_no",
    )
    .fetch_all(&state.pool)
    .await?;
    let students = rows
        .into_iter()
        .map(|r| StudentRow {
            id: r.get("id"),
            student_no: r.get("student_no"),
            name: r.get("name"),
            class_name: r.get("class_name"),
        })
        .collect();
    Ok(Json(StudentList { students }))
}

pub async fn import_csv(
    State(state): State<AppState>,
    Json(body): Json<ImportCsvBody>,
) -> Result<Json<ImportCsvResult>, AppError> {
    let parsed = parse_roster_csv(&body.csv);
    if parsed.header_error.is_some() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_CSV",
            parsed.header_error.unwrap(),
        ));
    }
    let mut upserted = 0usize;
    for row in &parsed.rows {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM students WHERE student_no = ?")
                .bind(&row.student_no)
                .fetch_optional(&state.pool)
                .await?;
        if let Some(id) = existing {
            sqlx::query("UPDATE students SET name = ?, class_name = ? WHERE id = ?")
                .bind(&row.name)
                .bind(&row.class_name)
                .bind(&id)
                .execute(&state.pool)
                .await?;
        } else {
            let id = Ulid::new().to_string();
            sqlx::query(
                "INSERT INTO students(id, student_no, name, class_name) VALUES(?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&row.student_no)
            .bind(&row.name)
            .bind(&row.class_name)
            .execute(&state.pool)
            .await?;
        }
        upserted += 1;
    }
    Ok(Json(ImportCsvResult {
        filename: body.filename,
        upserted,
        failed: parsed.failures.len(),
        failures: parsed.failures,
    }))
}

struct ParsedRow {
    student_no: String,
    name: String,
    class_name: String,
}

struct ParsedCsv {
    header_error: Option<String>,
    rows: Vec<ParsedRow>,
    failures: Vec<CsvFailure>,
}

fn parse_roster_csv(csv: &str) -> ParsedCsv {
    let mut lines = csv.lines().enumerate();
    let Some((_, header)) = lines.next() else {
        return ParsedCsv {
            header_error: Some("CSV 为空".into()),
            rows: vec![],
            failures: vec![],
        };
    };
    let cols: Vec<String> = split_csv_line(header)
        .into_iter()
        .map(|s| s.trim().trim_start_matches('\u{feff}').to_string())
        .collect();
    let class_i = find_col(&cols, &["班级", "class", "class_name"]);
    let no_i = find_col(&cols, &["学号", "student_no", "学号"]);
    let name_i = find_col(&cols, &["姓名", "name"]);
    if class_i.is_none() || no_i.is_none() || name_i.is_none() {
        return ParsedCsv {
            header_error: Some("表头必须含班级、学号、姓名".into()),
            rows: vec![],
            failures: vec![],
        };
    }
    let class_i = class_i.unwrap();
    let no_i = no_i.unwrap();
    let name_i = name_i.unwrap();
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for (idx, line) in lines {
        let line_no = idx + 1;
        if line.trim().is_empty() {
            continue;
        }
        let fields = split_csv_line(line);
        let class_name = fields.get(class_i).map(|s| s.trim().to_string()).unwrap_or_default();
        let student_no = fields.get(no_i).map(|s| s.trim().to_string()).unwrap_or_default();
        let name = fields.get(name_i).map(|s| s.trim().to_string()).unwrap_or_default();
        if class_name.is_empty() || student_no.is_empty() || name.is_empty() {
            failures.push(CsvFailure {
                line: line_no,
                reason: "缺少班级、学号或姓名".into(),
            });
            continue;
        }
        rows.push(ParsedRow {
            student_no,
            name,
            class_name,
        });
    }
    ParsedCsv {
        header_error: None,
        rows,
        failures,
    }
}

fn find_col(cols: &[String], aliases: &[&str]) -> Option<usize> {
    cols.iter().position(|c| aliases.iter().any(|a| c.eq_ignore_ascii_case(a)))
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                out.push(cur.trim().to_string());
                cur = String::new();
            }
            _ => cur.push(ch),
        }
    }
    out.push(cur.trim().to_string());
    out
}

pub async fn get_roster(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<RosterList>, AppError> {
    ensure_task_exists(&state, &task_id).await?;
    let rows = sqlx::query(
        r#"SELECT s.id, s.student_no, s.name, s.class_name
           FROM task_roster r
           JOIN students s ON s.id = r.student_id
           WHERE r.task_id = ?
           ORDER BY s.class_name, s.student_no"#,
    )
    .bind(&task_id)
    .fetch_all(&state.pool)
    .await?;
    let students: Vec<StudentRow> = rows
        .into_iter()
        .map(|r| StudentRow {
            id: r.get("id"),
            student_no: r.get("student_no"),
            name: r.get("name"),
            class_name: r.get("class_name"),
        })
        .collect();
    let student_ids = students.iter().map(|s| s.id.clone()).collect();
    Ok(Json(RosterList {
        student_ids,
        students,
    }))
}

pub async fn replace_roster(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(body): Json<RosterBody>,
) -> Result<Json<RosterList>, AppError> {
    ensure_task_exists(&state, &task_id).await?;
    let mut ids = body.student_ids;
    ids.sort();
    ids.dedup();
    let missing = missing_student_ids(&state, &ids).await?;
    if !missing.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "UNKNOWN_STUDENT",
            "只能从已导入名册中选择学生",
        )
        .with_details(json!({ "ids": missing })));
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM task_roster WHERE task_id = ?")
        .bind(&task_id)
        .execute(&mut *tx)
        .await?;
    for id in &ids {
        sqlx::query("INSERT INTO task_roster(task_id, student_id) VALUES(?, ?)")
            .bind(&task_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    get_roster(State(state), Path(task_id)).await
}

async fn ensure_task_exists(state: &AppState, task_id: &str) -> Result<(), AppError> {
    let found: Option<String> = sqlx::query_scalar("SELECT id FROM exam_tasks WHERE id = ?")
        .bind(task_id)
        .fetch_optional(&state.pool)
        .await?;
    if found.is_none() {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "考试任务不存在",
        ));
    }
    Ok(())
}

pub async fn missing_student_ids(
    state: &AppState,
    ids: &[String],
) -> Result<Vec<String>, AppError> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let sql = format!(
        "SELECT id FROM students WHERE id IN ({})",
        vec!["?"; ids.len()].join(",")
    );
    let mut q = sqlx::query_scalar::<_, String>(&sql);
    for id in ids {
        q = q.bind(id);
    }
    let found = q.fetch_all(&state.pool).await?;
    Ok(ids
        .iter()
        .filter(|id| !found.contains(id))
        .cloned()
        .collect())
}

pub async fn students_exist(state: &AppState, ids: &[String]) -> Result<bool, AppError> {
    Ok(missing_student_ids(state, ids).await?.is_empty())
}
