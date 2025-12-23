use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::net::TcpStream;
use hyper::client::conn::http1::SendRequest;
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;

/// HTTP 连接池
pub struct ConnectionPool {
    pools: Arc<Mutex<HashMap<SocketAddr, Vec<SendRequest<Incoming>>>>>,
}

impl ConnectionPool {
    pub fn new(_max_idle_per_host: usize) -> Self {
        Self {
            pools: Arc::new(Mutex::new(HashMap::new())),
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
    async fn create_connection(
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
}
