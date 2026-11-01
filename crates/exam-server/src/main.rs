use exam_server::{app, config::Config, db, AppState};

#[tokio::main]
async fn main() {
    let cfg = Config::from_env();
    cfg.ensure_data_dir().expect("create data dir");
    let pool = db::open_pool(&cfg.db_path()).await.expect("open sqlite");
    db::migrate(&pool).await.expect("migrate sqlite");
    let listener = tokio::net::TcpListener::bind(&cfg.bind)
        .await
        .unwrap_or_else(|e| panic!("bind {}: {e}", cfg.bind));
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(8080);
    axum::serve(listener, app(AppState { pool, port }))
        .await
        .expect("serve");
}
