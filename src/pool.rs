use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::Mutex;
use tokio::net::TcpStream;
use hyper::client::conn::http1::SendRequest;
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;

/// HTTP 连接池
pub struct ConnectionPool {
    pools: Mutex<HashMap<SocketAddr, Vec<SendRequest<Incoming>>>>,
    max_idle_per_host: usize,
}

impl ConnectionPool {
    pub fn new(max_idle_per_host: usize) -> Self {
        Self {
            pools: Mutex::new(HashMap::new()),
            max_idle_per_host, // 允许 0 表示不缓存空闲连接
        }
    }

    /// 获取或创建连接
    pub async fn get_or_create(
        &self,
        addr: SocketAddr,
    ) -> Result<SendRequest<Incoming>, std::io::Error> {
        // 尝试从池中获取
        {
            let mut pools = self.pools.lock().await;
            if let Some(pool) = pools.get_mut(&addr) {
                if let Some(sender) = pool.pop() {
                    // 检查连接是否仍然有效
                    if !sender.is_closed() {
                        return Ok(sender);
                    }
                }
            }
        }

        // 创建新连接
        self.create_connection(addr).await
    }
    /// 创建新的 HTTP 连接
    pub async fn create_connection(
        &self,
        addr: SocketAddr,
    ) -> Result<SendRequest<Incoming>, std::io::Error> {
        let stream = TcpStream::connect(addr).await?;
        let io = TokioIo::new(stream);

        let (sender, conn) = hyper::client::conn::http1::handshake(io)
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

        // 在后台运行连接
        tokio::spawn(async move {
            let _ = conn.await;
        });

        Ok(sender)
    }

    /// 回收连接到池中（带大小限制）
    pub async fn recycle(&self, addr: SocketAddr, sender: SendRequest<Incoming>) {
        // 检查连接是否仍然有效
        if sender.is_closed() {
            return;
        }

        let mut pools = self.pools.lock().await;
        let pool = pools.entry(addr).or_insert_with(Vec::new);

        // 只有在未达到上限时才回收连接
        if pool.len() < self.max_idle_per_host {
            pool.push(sender);
        }
        // 超过上限的连接会被自动丢弃
    }
}
