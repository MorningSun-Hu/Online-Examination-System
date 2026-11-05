use axum::body::Body;
use axum::http::{Request, StatusCode};
use exam_server::config::Config;
use exam_server::db::{migrate, open_pool};
use exam_server::{app, AppState};
use serde_json::{json, Value};
use tower::ServiceExt;

struct Harness {
    app: axum::Router,
    token: String,
    pool: sqlx::SqlitePool,
    _dir: tempfile::TempDir,
}

async fn json_body(res: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes) }))
}

async fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config::from_data_dir(dir.path());
    cfg.ensure_data_dir().unwrap();
    let pool = open_pool(&cfg.db_path()).await.unwrap();
    migrate(&pool).await.unwrap();
    let app = app(AppState {
        pool: pool.clone(),
        port: 8080,
    });
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/teacher/setup")
                .header("content-type", "application/json")
                .body(Body::from(json!({"password":"secret"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let token = json_body(res).await["token"].as_str().unwrap().to_string();
    Harness {
        app,
        token,
        pool,
        _dir: dir,
    }
}

fn auth_req(method: &str, uri: &str, token: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(body)
        .unwrap()
}

async fn insert_draft_task(pool: &sqlx::SqlitePool, id: &str) {
    sqlx::query(
        "INSERT INTO exam_tasks(id, title, mode, status, duration_minutes, allow_early_submit, total_score, pass_score)
         VALUES(?,'草稿','exam','draft',60,0,100,60)",
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn import_csv_upserts_skips_bad_rows_and_last_row_wins() {
    let h = harness().await;
    let csv = "\u{feff}班级,学号,姓名\n一班,001,张三\n一班,,缺学号\n一班,001,李四\n二班,002,王五\n";
    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "POST",
            "/api/teacher/students/import-csv",
            &h.token,
            Body::from(json!({"filename":"名册.csv","csv": csv}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let report = json_body(res).await;
    assert_eq!(report["upserted"], 3);
    assert_eq!(report["failed"], 1);
    assert_eq!(report["failures"][0]["line"], 3);

    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "GET",
            "/api/teacher/students",
            &h.token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let list = json_body(res).await;
    let students = list["students"].as_array().unwrap();
    assert_eq!(students.len(), 2);
    let s001 = students
        .iter()
        .find(|s| s["student_no"] == "001")
        .unwrap();
    assert_eq!(s001["name"], "李四");
    assert_eq!(s001["class_name"], "一班");
}

#[tokio::test]
async fn import_csv_rejects_missing_header_columns() {
    let h = harness().await;
    let res = h
        .app
        .oneshot(auth_req(
            "POST",
            "/api/teacher/students/import-csv",
            &h.token,
            Body::from(json!({"csv": "学号,姓名\n001,张三\n"}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let v = json_body(res).await;
    assert_eq!(v["error"]["code"], "BAD_CSV");
}

#[tokio::test]
async fn import_csv_accepts_tab_paste_without_header() {
    let h = harness().await;
    let csv = "高一1班\t2024001\t张三\n高一1班\t2024002\t李四\n";
    let res = h
        .app
        .oneshot(auth_req(
            "POST",
            "/api/teacher/students/import-csv",
            &h.token,
            Body::from(json!({"csv": csv}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let report = json_body(res).await;
    assert_eq!(report["upserted"], 2);
    assert_eq!(report["failed"], 0);
}

#[tokio::test]
async fn import_csv_accepts_fullwidth_comma_header() {
    let h = harness().await;
    let csv = "班级，学号，姓名\n二班，2024003，王五\n";
    let res = h
        .app
        .oneshot(auth_req(
            "POST",
            "/api/teacher/students/import-csv",
            &h.token,
            Body::from(json!({"csv": csv}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let report = json_body(res).await;
    assert_eq!(report["upserted"], 1);
}

#[tokio::test]
async fn roster_only_allows_imported_students() {
    let h = harness().await;
    insert_draft_task(&h.pool, "t1").await;

    let csv = "班级,学号,姓名\n一班,001,张三\n";
    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "POST",
            "/api/teacher/students/import-csv",
            &h.token,
            Body::from(json!({"csv": csv}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "GET",
            "/api/teacher/students",
            &h.token,
            Body::empty(),
        ))
        .await
        .unwrap();
    let sid = json_body(res).await["students"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "PUT",
            "/api/teacher/tasks/t1/roster",
            &h.token,
            Body::from(json!({"student_ids":["ghost"]}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let v = json_body(res).await;
    assert_eq!(v["error"]["code"], "UNKNOWN_STUDENT");

    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "PUT",
            "/api/teacher/tasks/missing/roster",
            &h.token,
            Body::from(json!({"student_ids":[sid]}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "PUT",
            "/api/teacher/tasks/t1/roster",
            &h.token,
            Body::from(json!({"student_ids":[sid]}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let roster = json_body(res).await;
    assert_eq!(roster["students"][0]["student_no"], "001");

    let res = h
        .app
        .oneshot(auth_req(
            "GET",
            "/api/teacher/tasks/t1/roster",
            &h.token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let got = json_body(res).await;
    assert_eq!(got["student_ids"][0], roster["students"][0]["id"]);
}
