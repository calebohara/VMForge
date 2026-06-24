//! Minimal QMP (QEMU Machine Protocol) client.
//!
//! QMP is newline-delimited JSON over a control socket. Protocol:
//! 1. Server sends a greeting `{"QMP": {...}}`.
//! 2. Client sends `{"execute":"qmp_capabilities"}` to leave negotiation mode.
//! 3. Commands `{"execute":..,"arguments":..}` get a `{"return":..}` or
//!    `{"error":..}`; async `{"event":..}` messages may arrive at any time.
//!
//! Phase 1 needs only sequential request/response (query-status, stop, cont,
//! system_powerdown, quit), so this client reads past events to the matching
//! reply. A fuller event-stream design lands when status push/events are wired.

use crate::error::{Error, Result};
use crate::model::VmState;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;

type Reader = BufReader<Box<dyn tokio::io::AsyncRead + Unpin + Send>>;
type Writer = Box<dyn tokio::io::AsyncWrite + Unpin + Send>;

pub struct QmpClient {
    reader: Reader,
    writer: Writer,
}

impl QmpClient {
    #[cfg(unix)]
    pub async fn connect_unix(path: &std::path::Path) -> Result<Self> {
        let stream = UnixStream::connect(path).await?;
        let (r, w) = stream.into_split();
        Self::from_halves(Box::new(r), Box::new(w)).await
    }

    pub async fn connect_tcp(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        let (r, w) = stream.into_split();
        Self::from_halves(Box::new(r), Box::new(w)).await
    }

    async fn from_halves(
        r: Box<dyn tokio::io::AsyncRead + Unpin + Send>,
        w: Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
    ) -> Result<Self> {
        let mut c = Self {
            reader: BufReader::new(r),
            writer: w,
        };
        c.handshake().await?;
        Ok(c)
    }

    async fn read_message(&mut self) -> Result<Value> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            return Err(Error::Qmp("QMP connection closed".into()));
        }
        serde_json::from_str(line.trim()).map_err(Error::from)
    }

    async fn write_message(&mut self, v: &Value) -> Result<()> {
        let mut s = serde_json::to_string(v)?;
        s.push('\n');
        self.writer.write_all(s.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }

    async fn handshake(&mut self) -> Result<()> {
        let greeting = self.read_message().await?;
        if greeting.get("QMP").is_none() {
            return Err(Error::Qmp(format!("unexpected greeting: {greeting}")));
        }
        self.execute("qmp_capabilities", None).await?;
        Ok(())
    }

    /// Execute a command, skipping async events, returning its `return` value.
    pub async fn execute(&mut self, cmd: &str, args: Option<Value>) -> Result<Value> {
        let req = match args {
            Some(a) => json!({ "execute": cmd, "arguments": a }),
            None => json!({ "execute": cmd }),
        };
        self.write_message(&req).await?;
        loop {
            let msg = self.read_message().await?;
            if msg.get("event").is_some() {
                tracing::debug!(target: "vmforge_core::qmp", event = %msg, "qmp event");
                continue;
            }
            if let Some(err) = msg.get("error") {
                return Err(Error::Qmp(err.to_string()));
            }
            if let Some(ret) = msg.get("return") {
                return Ok(ret.clone());
            }
            // greeting echo or unknown frame — ignore and keep reading.
        }
    }

    /// `query-status` mapped to a [`VmState`].
    pub async fn query_status(&mut self) -> Result<VmState> {
        let ret = self.execute("query-status", None).await?;
        let status = ret.get("status").and_then(Value::as_str).unwrap_or("");
        Ok(map_status(status))
    }

    /// Test-only placeholder client backed by a closed/empty stream. Used to
    /// populate a registry entry whose process is checked via `try_wait`
    /// (reaper) before any QMP I/O occurs. Never performs a handshake.
    #[cfg(test)]
    pub(crate) fn dummy() -> Self {
        Self {
            reader: BufReader::new(Box::new(tokio::io::empty())),
            writer: Box::new(tokio::io::sink()),
        }
    }
}

/// Map a QMP run-state string to VMForge's [`VmState`].
pub fn map_status(s: &str) -> VmState {
    match s {
        "running" => VmState::Running,
        "paused" | "suspended" | "watchdog" => VmState::Paused,
        "prelaunch" | "inmigrate" | "finish-migrate" => VmState::Starting,
        "shutdown" | "postmigrate" => VmState::Stopped,
        "internal-error" | "io-error" | "guest-panicked" => VmState::Error,
        _ => VmState::Running,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_run_states() {
        assert_eq!(map_status("running"), VmState::Running);
        assert_eq!(map_status("paused"), VmState::Paused);
        assert_eq!(map_status("prelaunch"), VmState::Starting);
        assert_eq!(map_status("shutdown"), VmState::Stopped);
        assert_eq!(map_status("guest-panicked"), VmState::Error);
    }
}
