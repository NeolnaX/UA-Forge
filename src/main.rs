mod config;
mod firewall;
mod handler;
mod lru;
mod logger;
mod pool;
mod server;
mod stats;
mod tproxy;

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use config::Config;

// 从 Cargo.toml 或环境变量获取版本号
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::from_args(std::env::args()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[uaforge] config error: {e}");
            return ExitCode::from(2);
        }
    };

    let _ = logger::init(
        logger::Level::parse(&config.log_level),
        config.log_file.as_deref(),
    );

    if config.show_version {
        println!("UAForge version: {VERSION}");
        return ExitCode::SUCCESS;
    }

    let stats = Arc::new(stats::Stats::new());
    stats.start_writer("/tmp/uaforge.stats", Duration::from_secs(5));

    let fw = Arc::new(firewall::FirewallManager::new(config.firewall.clone()));
    let handler = match handler::HttpHandler::new(config.clone(), stats.clone(), fw) {
        Ok(h) => Arc::new(h),
        Err(e) => {
            eprintln!("[uaforge] handler init error: {e}");
            return ExitCode::from(2);
        }
    };
    let server = server::Server::new(config, handler, stats);

    if let Err(e) = server.run().await {
        eprintln!("[uaforge] server error: {e}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
