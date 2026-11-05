use axum::extract::{Path, State};
use axum::Json;
use axum::extract::Query;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
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

#[derive(Deserialize, Default)]
pub struct StudentQuery {
    pub class_name: Option<String>,
}

#[derive(Deserialize)]
pub struct StudentIn {
    pub student_no: String,
    pub name: String,
    pub class_name: String,
}

#[derive(Serialize)]
pub struct ClassRow {
    pub id: String,
    pub name: String,
    pub student_count: i64,
}

#[derive(Deserialize)]
pub struct ClassIn {
    pub name: String,
}

pub async fn list_students(
    State(state): State<AppState>,
    Query(q): Query<StudentQuery>,
) -> Result<Json<StudentList>, AppError> {
    let rows = if let Some(class) = q.class_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        sqlx::query(
            "SELECT id, student_no, name, class_name FROM students WHERE class_name = ? ORDER BY student_no",
        )
        .bind(class)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, student_no, name, class_name FROM students ORDER BY class_name, student_no",
        )
        .fetch_all(&state.pool)
        .await?
    };
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
        ensure_class(&state, &row.class_name).await?;
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
    let normalized = csv.replace("\r\n", "\n").replace('\r', "\n");
    let all: Vec<&str> = normalized.lines().collect();
    let Some((header_idx, header)) = all
        .iter()
        .enumerate()
        .find(|(_, line)| !line.trim().is_empty())
    else {
        return ParsedCsv {
            header_error: Some("CSV 为空".into()),
            rows: vec![],
            failures: vec![],
        };
    };
    let delim = detect_delimiter(header);
    let cols: Vec<String> = split_csv_line(header, delim)
        .into_iter()
        .map(|s| s.trim().trim_start_matches('\u{feff}').to_string())
        .collect();
    let class_i = find_col(&cols, &["班级", "class", "class_name"]);
    let no_i = find_col(&cols, &["学号", "student_no"]);
    let name_i = find_col(&cols, &["姓名", "name"]);
    let headerless = class_i.is_none() || no_i.is_none() || name_i.is_none();
    if headerless && cols.len() < 3 {
        return ParsedCsv {
            header_error: Some("表头必须含班级、学号、姓名".into()),
            rows: vec![],
            failures: vec![],
        };
    }
    let (class_i, no_i, name_i, data_start) = if headerless {
        (0, 1, 2, header_idx)
    } else {
        (class_i.unwrap(), no_i.unwrap(), name_i.unwrap(), header_idx + 1)
    };
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for (idx, line) in all.iter().enumerate().skip(data_start) {
        let line_no = idx + 1;
        if line.trim().is_empty() {
            continue;
        }
        let fields = split_csv_line(line, delim);
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

fn detect_delimiter(header: &str) -> char {
    let tabs = header.matches('\t').count();
    if tabs > 0 {
        return '\t';
    }
    let commas = header.matches(',').count();
    let fullwidth = header.matches('，').count();
    let semis = header.matches(';').count();
    if fullwidth > commas && fullwidth >= semis {
        return '，';
    }
    if semis > commas {
        return ';';
    }
    ','
}

fn split_csv_line(line: &str, delim: char) -> Vec<String> {
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
            ch if ch == delim && !in_quotes => {
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
    let status: String = sqlx::query_scalar("SELECT status FROM exam_tasks WHERE id = ?")
        .bind(&task_id)
        .fetch_one(&state.pool)
        .await?;
    if status != "draft" {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "BAD_STATE",
            "发布后名单已冻结",
        ));
    }
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

pub async fn ensure_class(state: &AppState, name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "班级名称不能为空",
        ));
    }
    if let Some(id) = sqlx::query_scalar::<_, String>("SELECT id FROM classes WHERE name = ?")
        .bind(name)
        .fetch_optional(&state.pool)
        .await?
    {
        return Ok(id);
    }
    let id = Ulid::new().to_string();
    sqlx::query("INSERT INTO classes(id, name) VALUES(?, ?)")
        .bind(&id)
        .bind(name)
        .execute(&state.pool)
        .await?;
    Ok(id)
}

async fn sync_classes(state: &AppState) -> Result<(), AppError> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT class_name FROM students WHERE class_name NOT IN (SELECT name FROM classes)",
    )
    .fetch_all(&state.pool)
    .await?;
    for name in names {
        ensure_class(state, &name).await?;
    }
    Ok(())
}

pub async fn list_classes(State(state): State<AppState>) -> Result<Json<serde_json::Value>, AppError> {
    sync_classes(&state).await?;
    let rows = sqlx::query(
        r#"SELECT c.id, c.name,
                  (SELECT COUNT(*) FROM students s WHERE s.class_name = c.name) AS student_count
           FROM classes c ORDER BY c.name"#,
    )
    .fetch_all(&state.pool)
    .await?;
    let classes: Vec<ClassRow> = rows
        .into_iter()
        .map(|r| ClassRow {
            id: r.get("id"),
            name: r.get("name"),
            student_count: r.get("student_count"),
        })
        .collect();
    Ok(Json(json!({ "classes": classes })))
}

pub async fn create_class(
    State(state): State<AppState>,
    Json(body): Json<ClassIn>,
) -> Result<(StatusCode, Json<ClassRow>), AppError> {
    let id = ensure_class(&state, &body.name).await?;
    Ok((
        StatusCode::CREATED,
        Json(ClassRow {
            id,
            name: body.name.trim().to_string(),
            student_count: 0,
        }),
    ))
}

pub async fn rename_class(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ClassIn>,
) -> Result<StatusCode, AppError> {
    let new_name = body.name.trim();
    if new_name.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "班级名称不能为空",
        ));
    }
    let old: Option<String> = sqlx::query_scalar("SELECT name FROM classes WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await?;
    let old = old.ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "班级不存在"))?;
    if old != new_name {
        let clash: Option<String> = sqlx::query_scalar("SELECT id FROM classes WHERE name = ?")
            .bind(new_name)
            .fetch_optional(&state.pool)
            .await?;
        if clash.is_some() {
            return Err(AppError::new(
                StatusCode::CONFLICT,
                "DUPLICATE",
                "已有同名班级",
            ));
        }
        sqlx::query("UPDATE students SET class_name = ? WHERE class_name = ?")
            .bind(new_name)
            .bind(&old)
            .execute(&state.pool)
            .await?;
        sqlx::query("UPDATE classes SET name = ? WHERE id = ?")
            .bind(new_name)
            .bind(&id)
            .execute(&state.pool)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_class(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let name: Option<String> = sqlx::query_scalar("SELECT name FROM classes WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await?;
    let name = name.ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "班级不存在"))?;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM students WHERE class_name = ?")
        .bind(&name)
        .fetch_one(&state.pool)
        .await?;
    if n > 0 {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "IN_USE",
            "班级内还有学生，不能删除",
        ));
    }
    sqlx::query("DELETE FROM classes WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_student(
    State(state): State<AppState>,
    Json(body): Json<StudentIn>,
) -> Result<(StatusCode, Json<StudentRow>), AppError> {
    let student_no = body.student_no.trim();
    let name = body.name.trim();
    let class_name = body.class_name.trim();
    if student_no.is_empty() || name.is_empty() || class_name.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "班级、学号、姓名都不能为空",
        ));
    }
    ensure_class(&state, class_name).await?;
    let id = Ulid::new().to_string();
    sqlx::query("INSERT INTO students(id, student_no, name, class_name) VALUES(?, ?, ?, ?)")
        .bind(&id)
        .bind(student_no)
        .bind(name)
        .bind(class_name)
        .execute(&state.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE") {
                AppError::new(StatusCode::CONFLICT, "DUPLICATE", "学号已存在")
            } else {
                AppError::from(e)
            }
        })?;
    Ok((
        StatusCode::CREATED,
        Json(StudentRow {
            id,
            student_no: student_no.into(),
            name: name.into(),
            class_name: class_name.into(),
        }),
    ))
}

pub async fn update_student(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<StudentIn>,
) -> Result<StatusCode, AppError> {
    let student_no = body.student_no.trim();
    let name = body.name.trim();
    let class_name = body.class_name.trim();
    if student_no.is_empty() || name.is_empty() || class_name.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "班级、学号、姓名都不能为空",
        ));
    }
    ensure_class(&state, class_name).await?;
    let n = sqlx::query("UPDATE students SET student_no = ?, name = ?, class_name = ? WHERE id = ?")
        .bind(student_no)
        .bind(name)
        .bind(class_name)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE") {
                AppError::new(StatusCode::CONFLICT, "DUPLICATE", "学号已存在")
            } else {
                AppError::from(e)
            }
        })?
        .rows_affected();
    if n == 0 {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "学生不存在",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_student(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let papers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM papers WHERE student_id = ?")
        .bind(&id)
        .fetch_one(&state.pool)
        .await?;
    if papers > 0 {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "IN_USE",
            "该学生已有答卷，不能删除",
        ));
    }
    sqlx::query("DELETE FROM task_roster WHERE student_id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await?;
    let n = sqlx::query("DELETE FROM students WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "学生不存在",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn export_students(
    State(state): State<AppState>,
    Query(q): Query<StudentQuery>,
) -> Result<(HeaderMap, Vec<u8>), AppError> {
    let list = list_students(State(state), Query(q)).await?.0;
    let mut body = String::from("班级,学号,姓名\n");
    for s in list.students {
        body.push_str(&format!(
            "{},{},{}\n",
            csv_cell(&s.class_name),
            csv_cell(&s.student_no),
            csv_cell(&s.name)
        ));
    }
    Ok(csv_file("students.csv", body))
}

pub async fn students_template() -> (HeaderMap, Vec<u8>) {
    csv_file(
        "students-template.csv",
        "班级,学号,姓名\n一班,2024001,张三\n一班,2024002,李四\n".into(),
    )
}

pub async fn student_ids_for_classes(
    state: &AppState,
    names: &[String],
) -> Result<Vec<String>, AppError> {
    if names.is_empty() {
        return Ok(vec![]);
    }
    let sql = format!(
        "SELECT id FROM students WHERE class_name IN ({})",
        vec!["?"; names.len()].join(",")
    );
    let mut q = sqlx::query_scalar::<_, String>(&sql);
    for n in names {
        q = q.bind(n.trim());
    }
    Ok(q.fetch_all(&state.pool).await?)
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn csv_file(filename: &str, body: String) -> (HeaderMap, Vec<u8>) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    let disp = format!("attachment; filename=\"{filename}\"");
    if let Ok(v) = HeaderValue::from_str(&disp) {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(body.as_bytes());
    (headers, bytes)
}
