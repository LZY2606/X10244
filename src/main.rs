use saxs_bench::{casepack, server, store::Store};

#[tokio::main]
async fn main() {
    let mut addr = "127.0.0.1:5355".to_string();
    let mut db = "saxs.db".to_string();
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
    let mut store = Store::open(&db).expect("open sqlite db");
    if store.is_empty() {
        let log = casepack::replay(&mut store);
        println!("replayed offline case pack: {} events", log.len());
    }
    let app = server::router(store);
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    println!("SAXS bench listening on http://{}", addr);
    axum::serve(listener, app).await.unwrap();
}
