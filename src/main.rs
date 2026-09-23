use saxs_scattering_bench::{store::Store, web};
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let addr = parse_addr(std::env::args().skip(1).collect());
    let store = Store::open("saxs_bench.sqlite3").expect("打开数据库失败");
    store.migrate().expect("迁移失败");
    store.ensure_casepack().expect("载入离线案例包失败");

    let app = web::app(store);
    let listener = tokio::net::TcpListener::bind(addr).await.expect("绑定地址失败");
    println!("散射曲线台已启动：http://{}", listener.local().unwrap());
    axum::serve(listener, app).await.expect("服务失败");
}

fn parse_addr(args: Vec<String>) -> SocketAddr {
    let mut addr: SocketAddr = "127.0.0.1:5355".parse().unwrap();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--addr" && i + 1 < args.len() {
            addr = args[i + 1].parse().expect("无效地址");
            i += 2;
        } else {
            i += 1;
        }
    }
    addr
}
