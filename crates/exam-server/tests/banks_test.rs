use axum::body::Body;
use axum::http::{Request, StatusCode};
use exam_server::config::Config;
use exam_server::db::{migrate, open_pool};
use exam_server::{app, AppState};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tower::ServiceExt;

struct Harness {
    app: axum::Router,
    token: String,
    pool: SqlitePool,
    _dir: tempfile::TempDir,
}

fn testdata(name: &str) -> String {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("testdata/bank-html");
    p.push(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
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

#[tokio::test]
async fn create_list_and_edit_question() {
    let h = harness().await;
    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "POST",
            "/api/teacher/banks",
            &h.token,
            Body::from(json!({"name":"Python"}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let bank_id = json_body(res).await["id"].as_str().unwrap().to_string();

    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "POST",
            &format!("/api/teacher/banks/{bank_id}/questions"),
            &h.token,
            Body::from(
                json!({
                    "qtype": "single",
                    "stem": "1+1",
                    "options": [{"key":"A","text":"1"},{"key":"B","text":"2"}],
                    "answer": {"kind":"single","value":"B"}
                })
                .to_string(),
            ),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let qid = json_body(res).await["id"].as_str().unwrap().to_string();

    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "GET",
            "/api/teacher/banks",
            &h.token,
            Body::empty(),
        ))
        .await
        .unwrap();
    let list = json_body(res).await;
    assert_eq!(list["banks"][0]["question_count"], 1);
    assert_eq!(list["banks"][0]["name"], "Python");

    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "PUT",
            &format!("/api/teacher/questions/{qid}"),
            &h.token,
            Body::from(
                json!({
                    "qtype": "single",
                    "stem": "1+1=?",
                    "options": [{"key":"A","text":"1"},{"key":"B","text":"2"}],
                    "answer": {"kind":"single","value":"B"}
                })
                .to_string(),
            ),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let res = h
        .app
        .oneshot(auth_req(
            "GET",
            &format!("/api/teacher/banks/{bank_id}"),
            &h.token,
            Body::empty(),
        ))
        .await
        .unwrap();
    let detail = json_body(res).await;
    assert_eq!(detail["questions"][0]["stem"], "1+1=?");
}

#[tokio::test]
async fn import_export_and_partial() {
    let h = harness().await;
    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "POST",
            "/api/teacher/banks",
            &h.token,
            Body::from(json!({"name":"样例"}).to_string()),
        ))
        .await
        .unwrap();
    let bank_id = json_body(res).await["id"].as_str().unwrap().to_string();

    let html = testdata("spec-sample.html");
    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "POST",
            &format!("/api/teacher/banks/{bank_id}/import-html"),
            &h.token,
            Body::from(json!({"filename":"spec-sample.html","html": html}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let report = json_body(res).await;
    assert_eq!(report["imported"], 5);
    assert_eq!(report["failed"], 0);

    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "GET",
            &format!("/api/teacher/banks/{bank_id}/export-html"),
            &h.token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let exported = axum::body::to_bytes(res.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    let exported = String::from_utf8(exported.to_vec()).unwrap();
    assert!(exported.contains("lang=\"zh-CN\""));
    assert!(exported.contains("class=\"question\""));
    let again = bank_html::parse_html(&exported, "out.html").unwrap();
    assert_eq!(again.success.len(), 5);

    let empty = testdata("empty.html");
    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "POST",
            &format!("/api/teacher/banks/{bank_id}/import-html"),
            &h.token,
            Body::from(json!({"filename":"empty.html","html": empty}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let bank2 = h
        .app
        .clone()
        .oneshot(auth_req(
            "POST",
            "/api/teacher/banks",
            &h.token,
            Body::from(json!({"name":"部分"}).to_string()),
        ))
        .await
        .unwrap();
    let bank2_id = json_body(bank2).await["id"].as_str().unwrap().to_string();
    let partial = testdata("partial.html");
    let res = h
        .app
        .oneshot(auth_req(
            "POST",
            &format!("/api/teacher/banks/{bank2_id}/import-html"),
            &h.token,
            Body::from(json!({"filename":"partial.html","html": partial}).to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let report = json_body(res).await;
    assert_eq!(report["imported"], 1);
    assert_eq!(report["failed"], 1);
}

#[tokio::test]
async fn refuse_delete_when_published_snapshot_refs_question() {
    let h = harness().await;
    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "POST",
            "/api/teacher/banks",
            &h.token,
            Body::from(json!({"name":"锁"}).to_string()),
        ))
        .await
        .unwrap();
    let bank_id = json_body(res).await["id"].as_str().unwrap().to_string();
    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "POST",
            &format!("/api/teacher/banks/{bank_id}/questions"),
            &h.token,
            Body::from(
                json!({
                    "qtype": "judge",
                    "stem": "真",
                    "answer": {"kind":"judge","value": true}
                })
                .to_string(),
            ),
        ))
        .await
        .unwrap();
    let qid = json_body(res).await["id"].as_str().unwrap().to_string();

    sqlx::query("INSERT INTO students(id, student_no, name, class_name) VALUES('s1','1','张','一班')")
        .execute(&h.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO exam_tasks(id, title, mode, status, duration_minutes, allow_early_submit, total_score, pass_score)
         VALUES('t1','考','exam','published',60,0,100,60)",
    )
    .execute(&h.pool)
    .await
    .unwrap();
    let items = format!(r#"[{{"question_id":"{qid}","points":1}}]"#);
    sqlx::query(
        "INSERT INTO papers(id, task_id, student_id, attempt_no, items_json, created_at)
         VALUES('p1','t1','s1',1,?, '2026-11-01T00:00:00Z')",
    )
    .bind(&items)
    .execute(&h.pool)
    .await
    .unwrap();

    let res = h
        .app
        .clone()
        .oneshot(auth_req(
            "DELETE",
            &format!("/api/teacher/questions/{qid}"),
            &h.token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let v = json_body(res).await;
    assert_eq!(v["error"]["code"], "IN_USE");

    sqlx::query("UPDATE exam_tasks SET status='draft' WHERE id='t1'")
        .execute(&h.pool)
        .await
        .unwrap();
    let res = h
        .app
        .oneshot(auth_req(
            "DELETE",
            &format!("/api/teacher/questions/{qid}"),
            &h.token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
}
