use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;

use saxs_station::app::router;
use saxs_station::fixtures;
use saxs_station::store::Store;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--write-fixture") {
        let path = args
            .iter()
            .position(|arg| arg == "--write-fixture")
            .and_then(|index| args.get(index + 1))
            .map(String::as_str)
            .unwrap_or("cases/saxs-station/case.json");
        let json = serde_json::to_string_pretty(&fixtures::bundle())?;
        fs::write(path, json)?;
        println!("wrote {path}");
        return Ok(());
    }

    let addr: SocketAddr = args
        .iter()
        .position(|arg| arg == "--addr")
        .and_then(|index| args.get(index + 1))
        .unwrap_or(&"127.0.0.1:5355".to_string())
        .parse()?;
    let database_path =
        std::env::var("SAXS_SQLITE_PATH").unwrap_or_else(|_| "saxs_station.sqlite".to_string());
    let store = Arc::new(Store::open(&database_path)?);
    store.seed_case()?;

    let listener = TcpListener::bind(addr).await?;
    println!("SAXS station listening on http://{addr}");
    axum::serve(listener, router(store)).await?;
    Ok(())
}
