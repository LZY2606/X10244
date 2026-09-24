use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;

struct Args {
    addr: SocketAddr,
    db: PathBuf,
    reset: bool,
}

fn parse_args() -> Args {
    let mut addr: SocketAddr = "127.0.0.1:5355".parse().unwrap();
    let mut db = PathBuf::from("saxs.db");
    let mut reset = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--addr" => {
                let v = it.next().expect("--addr 需要值");
                addr = v.parse().expect("无法解析地址");
            }
            "--db" => db = it.next().expect("--db 需要值").into(),
            "--reset" => reset = true,
            other => panic!("未知参数 {other}"),
        }
    }
    Args { addr, db, reset }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args();
    if args.reset && args.db.exists() {
        std::fs::remove_file(&args.db).context("删除旧数据库失败")?;
    }
    let store = std::sync::Arc::new(tokio::sync::Mutex::new(store::Store::open(&args.db)?));
    let app = api::router(store);
    let listener = tokio::net::TcpListener::bind(args.addr).await?;
    eprintln!("SAXS 散射曲线台监听 http://{}", args.addr);
    axum::serve(listener, app).await?;
    Ok(())
}
