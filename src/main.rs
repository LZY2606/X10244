use std::sync::Mutex;

pub mod casepack;
pub mod graph;
pub mod model;
pub mod rules;
pub mod server;
pub mod store;

pub struct AppState {
    pub store: Mutex<store::Store>,
    pub db_path: String,
}

#[tokio::main]
async fn main() {
    let mut addr = "127.0.0.1:5355".to_string();
    let mut db = "saxs-bench.db".to_string();
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--addr" => {
                addr = args[i + 1].clone();
                i += 2;
            }
            "--db" => {
                db = args[i + 1].clone();
                i += 2;
            }
            _ => i += 1,
        }
    }
    let store = store::Store::open(&db).expect("open store");
    // first run: seed the offline case pack so the 90-second path works immediately
    if store.events().is_empty() {
        let summary = casepack::run_replay(&store);
        eprintln!("seeded case pack: {} steps", summary["steps"].as_array().unwrap().len());
    }
    let state = std::sync::Arc::new(AppState {
        store: Mutex::new(store),
        db_path: db,
    });
    let app = server::router(state);
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    eprintln!("saxs-bench listening on http://{}", addr);
    axum::serve(listener, app).await.unwrap();
}
