use saxs_station::server::{router, AppState};
use saxs_station::store::Store;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let mut addr = "127.0.0.1:5355".to_string();
    let mut db = "saxs.db".to_string();
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--addr" => {
                addr = args.get(i + 1).cloned().unwrap_or(addr);
                i += 1;
            }
            "--db" => {
                db = args.get(i + 1).cloned().unwrap_or(db);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    let store = Store::open(&db).expect("open db");
    let state = Arc::new(AppState { store });
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    println!("SAXS 散射曲线台 listening on http://{addr} (db: {db})");
    axum::serve(listener, app).await.unwrap();
}
