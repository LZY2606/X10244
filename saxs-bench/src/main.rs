//! Binary entry point. Defaults to a local SQLite file so the chain survives
//! restart; pass `--addr 127.0.0.1:5355` as documented.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let mut addr = "127.0.0.1:5355".to_string();
    let mut db = "saxs-bench.sqlite3".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--addr" => {
                if let Some(v) = args.next() {
                    addr = v;
                }
            }
            "--db" => {
                if let Some(v) = args.next() {
                    db = v;
                }
            }
            "-h" | "--help" => {
                println!("用法: saxs-bench --addr 127.0.0.1:5355 [--db saxs-bench.sqlite3]");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("未知参数: {other}");
                return ExitCode::FAILURE;
            }
        }
    }

    match saxs_bench::server::serve(&addr, &db).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("服务器启动失败: {e}");
            ExitCode::FAILURE
        }
    }
}
