//! VNC ↔ WebSocket bridge for the in-app noVNC console.
//!
//! QEMU's VNC server speaks RFB over raw TCP; noVNC speaks RFB over a
//! WebSocket. This is a byte-for-byte proxy (like `websockify`) implemented in
//! Rust so VMForge needs no Python dependency and the engine boundary stays
//! clean. Owned by console-engineer.

use crate::error::{Error, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

/// A running bridge. Dropping it aborts the accept loop.
pub struct VncBridge {
    /// Loopback WebSocket port noVNC connects to (`ws://127.0.0.1:<ws_port>`).
    pub ws_port: u16,
    handle: JoinHandle<()>,
}

impl Drop for VncBridge {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl VncBridge {
    /// Bind a loopback WebSocket server on an ephemeral port that proxies to
    /// the VM's VNC TCP port. Returns once the listener is bound.
    pub async fn start(vnc_port: u16) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let ws_port = listener.local_addr()?.port();
        let handle = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    if let Err(e) = proxy_conn(stream, vnc_port).await {
                        tracing::debug!(
                            target: "vmforge_core::console",
                            error = %e,
                            "vnc bridge connection ended"
                        );
                    }
                });
            }
        });
        Ok(Self { ws_port, handle })
    }
}

/// Pump bytes both ways between one WebSocket client and the VNC TCP server.
async fn proxy_conn(ws_stream: TcpStream, vnc_port: u16) -> Result<()> {
    let ws = tokio_tungstenite::accept_async(ws_stream)
        .await
        .map_err(|e| Error::Other(format!("ws handshake: {e}")))?;
    let tcp = TcpStream::connect(("127.0.0.1", vnc_port)).await?;
    let (mut tcp_r, mut tcp_w) = tcp.into_split();
    let (mut ws_w, mut ws_r) = ws.split();

    let to_tcp = async {
        while let Some(msg) = ws_r.next().await {
            match msg.map_err(|e| Error::Other(format!("ws recv: {e}")))? {
                Message::Binary(b) => tcp_w.write_all(&b).await?,
                Message::Close(_) => break,
                _ => {} // text/ping/pong: tungstenite auto-handles pings
            }
        }
        Ok::<(), Error>(())
    };

    let to_ws = async {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = tcp_r.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            ws_w.send(Message::Binary(buf[..n].to_vec().into()))
                .await
                .map_err(|e| Error::Other(format!("ws send: {e}")))?;
        }
        Ok::<(), Error>(())
    };

    tokio::select! {
        r = to_tcp => r?,
        r = to_ws => r?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::connect_async;

    #[tokio::test]
    async fn proxies_bytes_both_ways() {
        // Fake "VNC" server that echoes whatever it receives.
        let vnc = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let vnc_port = vnc.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = vnc.accept().await.unwrap();
            let mut b = [0u8; 1024];
            loop {
                let n = s.read(&mut b).await.unwrap();
                if n == 0 {
                    break;
                }
                s.write_all(&b[..n]).await.unwrap();
            }
        });

        let bridge = VncBridge::start(vnc_port).await.unwrap();
        let url = format!("ws://127.0.0.1:{}", bridge.ws_port);
        let (mut ws, _) = connect_async(url).await.unwrap();

        ws.send(Message::Binary(b"RFB 003.008\n".to_vec().into()))
            .await
            .unwrap();

        match ws.next().await {
            Some(Ok(Message::Binary(b))) => assert_eq!(&b[..], b"RFB 003.008\n"),
            other => panic!("expected echoed binary frame, got {other:?}"),
        }
    }
}
