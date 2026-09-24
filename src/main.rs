use std::net::SocketAddr;

use saxs_station::store::Store;
use saxs_station::web;

#[tokio::main]
async fn main() {
    let mut addr: SocketAddr = "127.0.0.1:5355".parse().unwrap();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--addr" => {
                if let Some(v) = args.next() {
                    addr = v.parse().expect("valid socket address");
                }
            }
            "--db" => {
                if let Some(v) = args.next() {
                    std::env::set_var("SAXS_DB", v);
                }
            }
            other => eprintln!("ignoring unknown argument: {other}"),
        }
    }
    let db = std::env::var("SAXS_DB").unwrap_or_else(|_| "saxs_station.sqlite".to_string());
    let store = Store::open(&db).expect("open sqlite store");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind addr");
    eprintln!("SAXS station listening on http://{addr} (db: {db})");
    axum::serve(listener, web::app(store)).await.unwrap();
}
