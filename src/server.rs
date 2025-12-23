use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::Request;
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;

use crate::config::Config;
use crate::handler::HttpHandler;
use crate::stats::Stats;
use crate::logger;
use crate::tproxy;
use crate::pool::ConnectionPool;

pub struct Server {
    config: Config,
    handler: Arc<HttpHandler>,
    stats: Arc<Stats>,
    pool: Arc<ConnectionPool>,
}

impl Server {
    pub fn new(config: Config, handler: Arc<HttpHandler>, stats: Arc<Stats>) -> Self {
        let pool = Arc::new(ConnectionPool::new(10)); // 每个主机最多10个空闲连接
        Self {
            config,
            handler,
            stats,
            pool,
        }
    }

    pub async fn run(&self) -> io::Result<()> {
        let addr = SocketAddr::from(([0, 0, 0, 0], self.config.port));
        let listener = TcpListener::bind(addr).await?;

        logger::log(
            logger::Level::Info,
            &format!("listening on {} (async mode)", addr),
        );

        loop {
            let (stream, _) = listener.accept().await?;

            let handler = self.handler.clone();
            let stats = self.stats.clone();
            let pool = self.pool.clone();

            // 为每个连接生成一个异步任务
            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, handler, stats, pool).await {
                    logger::log(
                        logger::Level::Debug,
                        &format!("connection error: {:?}", e)
                    );
                }
            });
        }
    }
}

/// 处理单个连接
async fn handle_connection(
    mut client: TcpStream,
    handler: Arc<HttpHandler>,
    stats: Arc<Stats>,
    pool: Arc<ConnectionPool>,
) -> Result<(), std::io::Error> {
    stats.add_active(1);
    let _guard = scopeguard::guard((), |_| stats.add_active(-1));

    // 获取原始目标地址
    let orig_dst = tproxy::original_dst_tokio(&client)?;
    let dest_ip = std::net::IpAddr::V4(*orig_dst.ip());
    let dest_port = orig_dst.port();

    logger::log(
        logger::Level::Debug,
        &format!("connection to {}:{}", dest_ip, dest_port)
    );

    // Peek 前几个字节检测是否是 HTTP
    let mut peek_buf = [0u8; 8];
    client.peek(&mut peek_buf).await?;

    let is_http = is_http_request(&peek_buf);

    if !is_http {
        // 非 HTTP 流量，报告给防火墙并直接转发
        handler.report_non_http(dest_ip, dest_port);

        logger::log(
            logger::Level::Debug,
            &format!("non-HTTP traffic to {}:{}, bypassing", dest_ip, dest_port)
        );

        // 连接到真实服务器并直接转发
        let mut server = TcpStream::connect(orig_dst).await?;
        tokio::io::copy_bidirectional(&mut client, &mut server).await?;
        return Ok(());
    }

    // HTTP 流量，使用 hyper 处理
    let server = TcpStream::connect(orig_dst).await?;
    process_http(client, server, handler, dest_ip, dest_port, pool).await
}

/// 使用 hyper 处理 HTTP 请求
async fn process_http(
    client: TcpStream,
    _server: TcpStream,
    handler: Arc<HttpHandler>,
    dest_ip: std::net::IpAddr,
    dest_port: u16,
    pool: Arc<ConnectionPool>,
) -> Result<(), std::io::Error> {
    // 使用 TokioIo 包装客户端连接
    let client_io = TokioIo::new(client);

    // 创建目标地址
    let dest_addr = std::net::SocketAddr::new(dest_ip, dest_port);

    let service = service_fn(move |req: Request<Incoming>| {
        let handler = handler.clone();
        let dest_addr = dest_addr.clone();
        let pool = pool.clone();
        async move {
            // 修改请求
            let modified_req = match handler.modify_request(req, dest_ip, dest_port).await {
                Ok(r) => r,
                Err(e) => {
                    return Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
                }
            };

            // 从连接池获取或创建连接
            let mut sender = pool.get_or_create(dest_addr)
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

            // 转发请求到真实服务器
            let response = sender.send_request(modified_req)
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

            // 回收连接到连接池
            pool.recycle(dest_addr, sender).await;

            Ok::<_, std::io::Error>(response)
        }
    });

    http1::Builder::new()
        .serve_connection(client_io, service)
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    Ok(())
}

/// 检测是否是 HTTP 请求
fn is_http_request(buf: &[u8]) -> bool {
    const HTTP_METHODS: &[&[u8]] = &[
        b"GET ", b"POST ", b"HEAD ", b"PUT ", 
        b"DELETE ", b"OPTIONS ", b"TRACE ", b"CONNECT ", b"PATCH "
    ];
    
    HTTP_METHODS.iter().any(|method| buf.starts_with(method))
}
