use std::net::SocketAddr;
use tokio::net::TcpStream;
use hyper::client::conn::http1::SendRequest;
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;

/// HTTP 连接器（每请求新建连接，确保协议正确性）
pub struct Connector;

impl Connector {
    pub fn new() -> Self {
        Self
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
}
