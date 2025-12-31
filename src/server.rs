use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
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

// 常量定义
const MAX_CONCURRENT_CONNECTIONS: usize = 10000;
const PEEK_BUFFER_SIZE: usize = 8;

pub struct Server {
    config: Config,
    handler: Arc<HttpHandler>,
    stats: Arc<Stats>,
    conn_limit: Arc<Semaphore>,
}

impl Server {
    pub fn new(config: Config, handler: Arc<HttpHandler>, stats: Arc<Stats>) -> Self {
        // 限制最大并发连接数，防止 DoS 资源耗尽
        let conn_limit = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
        Self {
            config,
            handler,
            stats,
            conn_limit,
        }
    }

    pub async fn run(&self) -> io::Result<()> {
        let addr = SocketAddr::from(([0, 0, 0, 0], self.config.port));
        let listener = TcpListener::bind(addr).await?;

        logger::log(
            logger::Level::Info,
            format_args!("listening on {} (async mode)", addr),
        );

        loop {
            let (stream, _) = listener.accept().await?;

            // 获取 permit，限制并发连接数
            let permit = match self.conn_limit.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => {
                    // Semaphore 被关闭，服务器正在关闭
                    logger::log(logger::Level::Info, format_args!("Semaphore closed, shutting down"));
                    return Ok(());
                }
            };

            let handler = self.handler.clone();
            let stats = self.stats.clone();

            // 为每个连接生成一个异步任务
            tokio::spawn(async move {
                let _permit = permit; // 持有 permit 直到连接结束
                if let Err(e) = handle_connection(stream, handler, stats).await {
                    logger::log(
                        logger::Level::Debug,
                        format_args!("connection error: {:?}", e)
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
) -> Result<(), std::io::Error> {
    stats.inc_active();
    let _guard = scopeguard::guard((), |_| stats.dec_active());

    // 获取原始目标地址
    let orig_dst = tproxy::original_dst_tokio(&client)?;
    let dest_ip = std::net::IpAddr::V4(*orig_dst.ip());
    let dest_port = orig_dst.port();

    logger::log(
        logger::Level::Debug,
        format_args!("connection to {}:{}", dest_ip, dest_port)
    );

    // Peek 前几个字节检测是否是 HTTP
    let mut peek_buf = [0u8; PEEK_BUFFER_SIZE];
    client.peek(&mut peek_buf).await?;

    let is_http = is_http_request(&peek_buf);

    if !is_http {
        // 非 HTTP 流量，报告给防火墙并直接转发
        handler.report_non_http(dest_ip, dest_port);

        logger::log(
            logger::Level::Debug,
            format_args!("non-HTTP traffic to {}:{}, bypassing", dest_ip, dest_port)
        );

        // 连接到真实服务器并直接转发
        let mut server = TcpStream::connect(orig_dst).await?;
        tokio::io::copy_bidirectional(&mut client, &mut server).await?;
        return Ok(());
    }

    // HTTP 流量，使用 hyper 处理
    process_http(client, handler, dest_ip, dest_port).await
}

/// 使用 hyper 处理 HTTP 请求
async fn process_http(
    client: TcpStream,
    handler: Arc<HttpHandler>,
    dest_ip: std::net::IpAddr,
    dest_port: u16,
) -> Result<(), std::io::Error> {
    // 使用 TokioIo 包装客户端连接
    let client_io = TokioIo::new(client);

    // 创建目标地址
    let dest_addr = std::net::SocketAddr::new(dest_ip, dest_port);

    let service = service_fn(move |req: Request<Incoming>| {
        let handler = handler.clone();
        async move {
            // 修改请求
            let modified_req = match handler.modify_request(req, dest_ip, dest_port).await {
                Ok(r) => r,
                Err(e) => {
                    return Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
                }
            };

            // 直接创建新连接（每请求新建，确保 HTTP/1.1 协议正确性）
            let stream = TcpStream::connect(dest_addr).await?;
            let io = TokioIo::new(stream);

            let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

            // 在后台运行连接
            tokio::spawn(async move {
                let _ = conn.await;
            });

            // 转发请求到真实服务器
            let response = sender.send_request(modified_req)
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

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
