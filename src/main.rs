use axum::serve;
use saxs_scattering_desk::api::build_router;
use saxs_scattering_desk::service::AppState;
use std::env;
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let mut addr: SocketAddr = "127.0.0.1:5355".parse()?;
    let mut database = "saxs-desk.sqlite".to_string();
    let mut seed = true;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--addr" => {
                index += 1;
                addr = args[index].parse()?;
            }
            "--db" => {
                index += 1;
                database = args[index].clone();
            }
            "--no-seed" => seed = false,
            _ => {}
        }
        index += 1;
    }
    let state = AppState::new(&database);
    if seed && state.events().is_empty() {
        state.storage.lock().unwrap().replace_with_events(service::seeded_events())?;
    }
    let listener = TcpListener::bind(addr).await?;
    println!("SAXS scattering desk listening on http://{addr}");
    serve(listener, saxs_scattering_desk::api::build_router_from_state(state)).await?;
    Ok(())
}
