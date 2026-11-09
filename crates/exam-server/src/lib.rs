pub mod config;
pub mod db;
pub mod error;
pub mod auth;
pub mod lan;
pub mod banks;
pub mod students;
pub mod paper;
pub mod papers;
pub mod tasks;
pub mod student;
pub mod results;

use axum::middleware;
use axum::routing::{get, post, put};
use axum::routing::patch;
use axum::Router;
use sqlx::SqlitePool;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub port: u16,
}

pub fn app(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/teacher/password", put(auth::change_password))
        .route("/api/teacher/lan-addresses", get(lan::lan_addresses))
        .route("/api/teacher/banks", get(banks::list_banks).post(banks::create_bank))
        .route(
            "/api/teacher/banks/{id}",
            get(banks::get_bank)
                .patch(banks::rename_bank)
                .delete(banks::delete_bank),
        )
        .route(
            "/api/teacher/banks/{id}/questions",
            post(banks::create_question),
        )
        .route(
            "/api/teacher/questions/{id}",
            put(banks::update_question).delete(banks::delete_question),
        )
        .route(
            "/api/teacher/banks/{id}/import-html",
            post(banks::import_html),
        )
        .route(
            "/api/teacher/banks/{id}/export-html",
            get(banks::export_html),
        )
        .route("/api/teacher/students", get(students::list_students).post(students::create_student))
        .route(
            "/api/teacher/students/import-csv",
            post(students::import_csv),
        )
        .route(
            "/api/teacher/students/{id}",
            patch(students::update_student).delete(students::delete_student),
        )
        .route("/api/teacher/students.csv", get(students::export_students))
        .route(
            "/api/teacher/student-template.csv",
            get(students::students_template),
        )
        .route(
            "/api/teacher/classes",
            get(students::list_classes).post(students::create_class),
        )
        .route(
            "/api/teacher/classes/{id}",
            patch(students::rename_class).delete(students::delete_class),
        )
        .route(
            "/api/teacher/papers",
            get(papers::list_papers).post(papers::create_paper),
        )
        .route(
            "/api/teacher/papers/{id}",
            get(papers::get_paper).patch(papers::patch_paper).delete(papers::delete_paper),
        )
        .route(
            "/api/teacher/papers/{id}/preview",
            get(papers::preview_paper),
        )
        .route(
            "/api/teacher/tasks/{id}/roster",
            get(students::get_roster).put(students::replace_roster),
        )
        .route("/api/teacher/tasks", get(tasks::list_tasks).post(tasks::create_task))
        .route(
            "/api/teacher/tasks/{id}",
            get(tasks::get_task).patch(tasks::patch_task).delete(tasks::delete_task),
        )
        .route("/api/teacher/tasks/{id}/split", post(tasks::split_by_class))
        .route("/api/teacher/tasks/{id}/publish", post(tasks::publish))
        .route("/api/teacher/tasks/{id}/close", post(tasks::close))
        .route("/api/teacher/tasks/{id}/end", post(tasks::end))
        .route("/api/teacher/tasks/{id}/grade", post(tasks::confirm_grade))
        .route("/api/teacher/tasks/{id}/release", post(tasks::release))
        .route("/api/teacher/tasks/{id}/monitor", get(tasks::monitor))
        .route("/api/teacher/tasks/{id}/results", get(results::list_results))
        .route("/api/teacher/tasks/{id}/results.csv", get(results::export_csv))
        .route("/api/teacher/tasks/{id}/analysis", get(results::analysis))
        .route("/api/teacher/attempts/{id}", get(results::attempt_paper))
        .route(
            "/api/teacher/attempts/{id}/code-scores",
            put(results::code_scores),
        )
        .route(
            "/api/teacher/attempts/{id}/confirm-marking",
            post(results::confirm_marking),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::teacher_auth,
        ));
    let student_ok = Router::new()
        .route("/api/student/paper", get(student::paper))
        .route("/api/student/answers", put(student::save_answers))
        .route("/api/student/submit", post(student::submit))
        .route("/api/student/result", get(student::result))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            student::student_auth,
        ));
    let router = Router::new()
        .route("/api/health", get(health))
        .route("/api/teacher/setup", post(auth::setup))
        .route("/api/teacher/login", post(auth::login))
        .route("/api/student/enter", post(student::enter))
        .route("/api/student/brief", get(student::brief))
        .route("/api/student/lookup", get(student::lookup))
        .merge(protected)
        .merge(student_ok)
        .with_state(state);
    if let Some(dir) = find_web_dir() {
        router
            .route(
                "/",
                get(|| async { axum::response::Redirect::temporary("/teacher.html") }),
            )
            .route(
                "/teacher",
                get(|| async { axum::response::Redirect::temporary("/teacher.html") }),
            )
            .route(
                "/student",
                get(|| async { axum::response::Redirect::temporary("/student.html") }),
            )
            .fallback_service(tower_http::services::ServeDir::new(dir))
    } else {
        router
    }
}

fn find_web_dir() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("EXAM_WEB_DIR") {
        let p = std::path::PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    for c in ["web/dist", "./web/dist", "web"] {
        let p = std::path::PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let p = parent.join("web");
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

async fn health() -> &'static str {
    "ok"
}
