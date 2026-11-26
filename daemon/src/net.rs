use anyhow::{Result, bail, ensure};
use std::{future::Future, io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
};

pub async fn serve_tcp<F, Fut>(listener: TcpListener, handler: F)
where
    F: Fn(TcpStream, SocketAddr) -> Fut,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    let slots = Arc::new(Semaphore::new(128));
    loop {
        let permit = slots.clone().acquire_owned().await.unwrap();
        match listener.accept().await {
            Ok((stream, peer)) => {
                let future = handler(stream, peer);
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ = future.await;
                });
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}
/// Read one delimiter-terminated frame without allocating beyond the configured limit.
pub async fn frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    end: &[u8],
    limit: usize,
) -> Result<Vec<u8>> {
    let mut frame = Vec::new();
    loop {
        let buf = reader.fill_buf().await?;
        if buf.is_empty() {
            if frame.is_empty() {
                return Ok(frame);
            }
            bail!(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        let size = buf.len().min(limit - frame.len());
        let previous = frame.len();
        frame.extend_from_slice(&buf[..size]);
        if let Some(pos) = frame[previous.saturating_sub(end.len() - 1)..]
            .windows(end.len())
            .position(|w| w == end)
        {
            let len = previous.saturating_sub(end.len() - 1) + pos + end.len();
            reader.consume(len - previous);
            frame.truncate(len);
            return Ok(frame);
        }
        reader.consume(size);
        ensure!(frame.len() < limit, "frame exceeds {limit} bytes");
    }
}
pub fn reachable(mut addr: SocketAddr) -> SocketAddr {
    if addr.ip().is_unspecified() {
        addr.set_ip(if addr.is_ipv4() {
            std::net::Ipv4Addr::LOCALHOST.into()
        } else {
            std::net::Ipv6Addr::LOCALHOST.into()
        });
    }
    addr
}
