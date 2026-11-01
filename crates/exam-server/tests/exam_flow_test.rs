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
    std::mem::forget(dir);
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
    Harness { app, token }
}

fn auth(method: &str, uri: &str, token: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(body)
        .unwrap()
}

fn student_req(method: &str, uri: &str, token: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(body)
        .unwrap()
}

async fn call(app: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    (status, json_body(res).await)
}

async fn seed_bank(h: &Harness) -> String {
    let (st, v) = call(
        &h.app,
        auth(
            "POST",
            "/api/teacher/banks",
            &h.token,
            Body::from(json!({"name":"基础"}).to_string()),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let bank_id = v["id"].as_str().unwrap().to_string();
    for i in 0..4 {
        let (st, _) = call(
            &h.app,
            auth(
                "POST",
                &format!("/api/teacher/banks/{bank_id}/questions"),
                &h.token,
                Body::from(
                    json!({
                        "qtype": "single",
                        "stem": format!("单选{i}"),
                        "options": [
                            {"key":"A","text":"a"},
                            {"key":"B","text":"b"},
                            {"key":"C","text":"c"},
                            {"key":"D","text":"d"}
                        ],
                        "answer": {"kind":"single","value":"B"}
                    })
                    .to_string(),
                ),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED);
    }
    let (st, _) = call(
        &h.app,
        auth(
            "POST",
            &format!("/api/teacher/banks/{bank_id}/questions"),
            &h.token,
            Body::from(
                json!({
                    "qtype": "judge",
                    "stem": "判断题",
                    "answer": {"kind":"judge","value": true}
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    bank_id
}

async fn seed_students(h: &Harness) -> Vec<String> {
    let csv = "班级,学号,姓名\n一班,001,张三\n一班,002,张三\n一班,003,李四\n";
    let (st, _) = call(
        &h.app,
        auth(
            "POST",
            "/api/teacher/students/import-csv",
            &h.token,
            Body::from(json!({"csv": csv}).to_string()),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, v) = call(
        &h.app,
        auth("GET", "/api/teacher/students", &h.token, Body::empty()),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    v["students"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect()
}

async fn create_ok_task(h: &Harness, bank_id: &str, ids: &[String], mode: &str, early: bool) -> String {
    let (st, v) = call(
        &h.app,
        auth(
            "POST",
            "/api/teacher/tasks",
            &h.token,
            Body::from(
                json!({
                    "title": "测验",
                    "mode": mode,
                    "duration_minutes": 60,
                    "allow_early_submit": early,
                    "total_score": 10,
                    "student_ids": ids,
                    "rule": {
                        "bank_ids": [bank_id],
                        "type_counts": {"single": 2, "judge": 1},
                        "shuffle_within_section": true,
                        "shuffle_options": true
                    }
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn create_task_not_enough_keeps_draft() {
    let h = harness().await;
    let bank_id = seed_bank(&h).await;
    let ids = seed_students(&h).await;
    let (st, v) = call(
        &h.app,
        auth(
            "POST",
            "/api/teacher/tasks",
            &h.token,
            Body::from(
                json!({
                    "title": "不足",
                    "mode": "exam",
                    "duration_minutes": 30,
                    "allow_early_submit": true,
                    "total_score": 100,
                    "student_ids": ids,
                    "rule": {
                        "bank_ids": [bank_id],
                        "type_counts": {"single": 40}
                    }
                })
                .to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(v["error"]["code"], "NOT_ENOUGH");
    let tid = v["error"]["details"]["task_id"].as_str().unwrap();
    let (st, t) = call(
        &h.app,
        auth(
            "GET",
            &format!("/api/teacher/tasks/{tid}"),
            &h.token,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(t["status"], "draft");
}

#[tokio::test]
async fn exam_enter_paper_submit_and_release() {
    let h = harness().await;
    let bank_id = seed_bank(&h).await;
    let ids = seed_students(&h).await;
    let task_id = create_ok_task(&h, &bank_id, &ids, "exam", false).await;
    let (st, _) = call(
        &h.app,
        auth(
            "POST",
            &format!("/api/teacher/tasks/{task_id}/publish"),
            &h.token,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    let (st, v) = call(
        &h.app,
        Request::builder()
            .method("POST")
            .uri("/api/student/enter")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "class_name":"一班",
                    "name":"李四",
                    "client_id":"pc-1",
                    "task_id": task_id
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let tok = v["token"].as_str().unwrap().to_string();
    let paper = serde_json::to_string(&v).unwrap();
    assert!(!paper.contains("\"answer\""), "考试公开前不能含答案: {paper}");
    assert!(v["items"][0].get("points").is_none());

    let qid = v["items"][0]["question_id"].as_str().unwrap().to_string();
    let (st, err) = call(
        &h.app,
        student_req("POST", "/api/student/submit", &tok, Body::empty()),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert_eq!(err["error"]["code"], "EARLY_SUBMIT");

    let (st, _) = call(
        &h.app,
        auth(
            "PATCH",
            &format!("/api/teacher/tasks/{task_id}"),
            &h.token,
            Body::from(json!({"allow_early_submit": true}).to_string()),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let (st, _) = call(
        &h.app,
        student_req(
            "PUT",
            "/api/student/answers",
            &tok,
            Body::from(
                json!({"answers": { qid.clone(): {"kind":"single","value":"B"} }}).to_string(),
            ),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let (st, sub) = call(
        &h.app,
        student_req("POST", "/api/student/submit", &tok, Body::empty()),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{sub}");
    assert_eq!(sub["status"], "submitted");
    assert!(sub.get("items").is_none());

    let (st, again) = call(
        &h.app,
        student_req("POST", "/api/student/submit", &tok, Body::empty()),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(again["error"]["code"], "SUBMITTED");

    let (st, mon) = call(
        &h.app,
        auth(
            "GET",
            &format!("/api/teacher/tasks/{task_id}/monitor"),
            &h.token,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let statuses: Vec<&str> = mon["students"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["status"].as_str().unwrap())
        .collect();
    assert!(statuses.contains(&"submitted"));
    assert!(statuses.contains(&"not_entered"));

    let (st, _) = call(
        &h.app,
        auth(
            "POST",
            &format!("/api/teacher/tasks/{task_id}/close"),
            &h.token,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    let (st, _) = call(
        &h.app,
        auth(
            "POST",
            &format!("/api/teacher/tasks/{task_id}/release"),
            &h.token,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    let (st, res) = call(
        &h.app,
        student_req("GET", "/api/student/result", &tok, Body::empty()),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{res}");
    assert_eq!(res["delivery"], "online");
    assert!(res["items"][0].get("answer").is_some());

    let (st, csv) = call(
        &h.app,
        auth(
            "GET",
            &format!("/api/teacher/tasks/{task_id}/results.csv"),
            &h.token,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let raw = csv["raw"].as_str().unwrap_or("");
    assert!(raw.contains("学号") || csv.to_string().contains("李四"));

    let (st, ana) = call(
        &h.app,
        auth(
            "GET",
            &format!("/api/teacher/tasks/{task_id}/analysis"),
            &h.token,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{ana}");
    assert!(ana["count"].as_i64().unwrap() >= 1);
}

#[tokio::test]
async fn student_match_rules() {
    let h = harness().await;
    let bank_id = seed_bank(&h).await;
    let ids = seed_students(&h).await;
    let task_id = create_ok_task(&h, &bank_id, &ids, "exam", true).await;
    let (st, _) = call(
        &h.app,
        auth(
            "POST",
            &format!("/api/teacher/tasks/{task_id}/publish"),
            &h.token,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    let (st, v) = call(
        &h.app,
        Request::builder()
            .method("POST")
            .uri("/api/student/enter")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"class_name":"一班","name":"不存在","client_id":"x","task_id":task_id})
                    .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert_eq!(v["error"]["code"], "NO_MATCH");

    let (st, v) = call(
        &h.app,
        Request::builder()
            .method("POST")
            .uri("/api/student/enter")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"class_name":"一班","name":"张三","client_id":"x","task_id":task_id})
                    .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(v["error"]["code"], "DUPLICATE_NAME");

    let (st, v) = call(
        &h.app,
        Request::builder()
            .method("POST")
            .uri("/api/student/enter")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"class_name":"一班","student_no":"001","name":"李四","client_id":"x","task_id":task_id})
                    .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert_eq!(v["error"]["code"], "NAME_MISMATCH");
}

#[tokio::test]
async fn practice_resit_and_offline_release() {
    let h = harness().await;
    let bank_id = seed_bank(&h).await;
    let ids = seed_students(&h).await;
    let task_id = create_ok_task(&h, &bank_id, &ids, "practice", true).await;
    let (st, _) = call(
        &h.app,
        auth(
            "POST",
            &format!("/api/teacher/tasks/{task_id}/publish"),
            &h.token,
            Body::empty(),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    let enter = |cid: &str| {
        Request::builder()
            .method("POST")
            .uri("/api/student/enter")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "class_name":"一班",
                    "student_no":"003",
                    "client_id": cid,
                    "task_id": task_id
                })
                .to_string(),
            ))
            .unwrap()
    };

    let (st, v) = call(&h.app, enter("pc-a")).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let tok1 = v["token"].as_str().unwrap().to_string();
    let paper1 = v["items"].clone();
    let (st, sub) = call(
        &h.app,
        student_req("POST", "/api/student/submit", &tok1, Body::empty()),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{sub}");
    assert!(sub["items"][0].get("answer").is_some());

    let (st, v2) = call(&h.app, enter("pc-a")).await;
    assert_eq!(st, StatusCode::OK, "{v2}");
    assert_eq!(v2["attempt_no"], 2);
    assert_ne!(v2["items"], paper1);
}
