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
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio::time::Instant;

/// One entry from QMP `query-jobs`. Extra fields are ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct JobInfo {
    pub id: String,
    pub status: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub error: Option<String>,
}

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

    /// `query-jobs` → the current list of background jobs.
    pub async fn query_jobs(&mut self) -> Result<Vec<JobInfo>> {
        let ret = self.execute("query-jobs", None).await?;
        let jobs: Vec<JobInfo> = serde_json::from_value(ret)?;
        Ok(jobs)
    }

    /// Queue a background job command (e.g. `snapshot-save`/`snapshot-load`/
    /// `snapshot-delete`), then poll `query-jobs` until the job identified by
    /// `job_id` reaches `concluded` (or disappears), finally `job-dismiss`-ing
    /// it. A non-null job `error` maps to [`Error::Qmp`]. The whole operation
    /// is bounded by `timeout`.
    ///
    /// QEMU job lifecycle for these commands: created → running → … →
    /// `concluded` (terminal). On failure the `error` field is set while the
    /// status is `concluded`/`aborting`. A `concluded` job persists until
    /// dismissed (auto-dismiss is off for these), so we always dismiss.
    pub async fn run_job(
        &mut self,
        cmd: &str,
        args: Value,
        job_id: &str,
        timeout: Duration,
    ) -> Result<()> {
        // Queue the job. The immediate `{"return":{}}` only means "accepted".
        self.execute(cmd, Some(args)).await?;

        let deadline = Instant::now() + timeout;
        let poll_interval = Duration::from_millis(200);
        loop {
            if Instant::now() >= deadline {
                return Err(Error::Qmp(format!(
                    "job {job_id} did not conclude within {timeout:?}"
                )));
            }

            let jobs = self.query_jobs().await?;
            match jobs.iter().find(|j| j.id == job_id) {
                Some(job) => {
                    if let Some(err) = &job.error {
                        if !err.is_empty() {
                            // Best-effort dismiss of the failed job, then surface.
                            let _ = self
                                .execute("job-dismiss", Some(json!({"id": job_id})))
                                .await;
                            return Err(Error::Qmp(format!("job {job_id} failed: {err}")));
                        }
                    }
                    if job.status == "concluded" {
                        // Success: dismiss and finish.
                        self.execute("job-dismiss", Some(json!({"id": job_id})))
                            .await?;
                        return Ok(());
                    }
                    // Still running/created/etc — wait and re-poll.
                }
                // Job already absent (auto-dismissed / never visible) → done.
                None => return Ok(()),
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            tokio::time::sleep(poll_interval.min(remaining)).await;
        }
    }

    /// Test-only placeholder client backed by a closed/empty stream. Used to
    /// populate a registry entry whose process is checked via `try_wait`
    /// (reaper) before any QMP I/O occurs. Never performs a handshake. Only the
    /// unix-gated engine reaper tests use it, so gate it to match (avoids a
    /// dead-code warning on the Windows test build).
    #[cfg(all(test, unix))]
    pub(crate) fn dummy() -> Self {
        Self {
            reader: BufReader::new(Box::new(tokio::io::empty())),
            writer: Box::new(tokio::io::sink()),
        }
    }

    /// Test-only client driven by a scripted, in-memory reply stream. Each
    /// value in `replies` is one newline-delimited QMP frame the fake server
    /// will hand back, in order, as the client reads. No handshake is run —
    /// the script feeds replies directly to `execute`/`query_jobs`/`run_job`.
    /// Writes go to a sink (the request bytes are not inspected).
    #[cfg(test)]
    pub(crate) fn scripted(replies: &[Value]) -> Self {
        let mut buf = String::new();
        for r in replies {
            buf.push_str(&serde_json::to_string(r).unwrap());
            buf.push('\n');
        }
        // `std::io::Cursor<Vec<u8>>` implements `tokio::io::AsyncRead` via
        // tokio's blanket impl for `AsRef<[u8]>`.
        let reader: Box<dyn tokio::io::AsyncRead + Unpin + Send> =
            Box::new(std::io::Cursor::new(buf.into_bytes()));
        Self {
            reader: BufReader::new(reader),
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

    // ---- run_job via a scripted in-memory reply stream (§E) ----

    // Success path: queue accepted ({"return":{}}), one query-jobs reply with
    // the job "concluded" and no error → run_job dismisses ({"return":{}}) and
    // returns Ok.
    #[tokio::test]
    async fn run_job_concludes_then_dismisses() {
        let mut c = QmpClient::scripted(&[
            // 1. response to the queued snapshot-save command
            json!({"return": {}}),
            // 2. response to query-jobs: job concluded, no error
            json!({"return": [
                {"id": "job-1", "type": "snapshot-save", "status": "concluded", "error": null}
            ]}),
            // 3. response to job-dismiss
            json!({"return": {}}),
        ]);
        let res = c
            .run_job(
                "snapshot-save",
                json!({"job-id": "job-1", "tag": "t", "vmstate": "disk0", "devices": ["disk0"]}),
                "job-1",
                Duration::from_secs(5),
            )
            .await;
        assert!(res.is_ok(), "expected Ok, got {res:?}");
    }

    // Error path: query-jobs reports a job with a non-null error → run_job maps
    // it to Err(Qmp). (The scripted reader also has a dismiss reply queued for
    // the best-effort job-dismiss the error branch issues.)
    #[tokio::test]
    async fn run_job_error_maps_to_qmp_err() {
        let mut c = QmpClient::scripted(&[
            json!({"return": {}}),
            json!({"return": [
                {"id": "job-1", "type": "snapshot-save", "status": "concluded",
                 "error": "Failed to save vmstate"}
            ]}),
            json!({"return": {}}),
        ]);
        let res = c
            .run_job(
                "snapshot-save",
                json!({"job-id": "job-1"}),
                "job-1",
                Duration::from_secs(5),
            )
            .await;
        assert!(
            matches!(res, Err(Error::Qmp(_))),
            "expected Err(Qmp), got {res:?}"
        );
    }

    // A job that is already absent from query-jobs is treated as concluded.
    #[tokio::test]
    async fn run_job_absent_job_is_ok() {
        let mut c = QmpClient::scripted(&[json!({"return": {}}), json!({"return": []})]);
        let res = c
            .run_job(
                "snapshot-delete",
                json!({"job-id": "gone"}),
                "gone",
                Duration::from_secs(5),
            )
            .await;
        assert!(res.is_ok(), "absent job => Ok, got {res:?}");
    }

    // snapshot-load run_job concludes then dismisses (suspend/resume path, §E).
    // Same scripted harness as snapshot-save: queue accepted → one query-jobs
    // reply with the job "concluded" and no error → dismiss → Ok.
    #[tokio::test]
    async fn run_job_snapshot_load_concludes_then_dismisses() {
        let mut c = QmpClient::scripted(&[
            // 1. response to the queued snapshot-load command
            json!({"return": {}}),
            // 2. response to query-jobs: job concluded, no error
            json!({"return": [
                {"id": "job-load", "type": "snapshot-load", "status": "concluded", "error": null}
            ]}),
            // 3. response to job-dismiss
            json!({"return": {}}),
        ]);
        let res = c
            .run_job(
                "snapshot-load",
                json!({"job-id": "job-load", "tag": "t", "vmstate": "disk0", "devices": ["disk0"]}),
                "job-load",
                Duration::from_secs(5),
            )
            .await;
        assert!(res.is_ok(), "expected Ok, got {res:?}");
    }

    // query-jobs parses a JobInfo list with the type rename.
    #[tokio::test]
    async fn query_jobs_parses_job_info() {
        let mut c = QmpClient::scripted(&[json!({"return": [
            {"id": "j", "type": "snapshot-load", "status": "running"}
        ]})]);
        let jobs = c.query_jobs().await.expect("query-jobs");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "j");
        assert_eq!(jobs[0].kind, "snapshot-load");
        assert_eq!(jobs[0].status, "running");
        assert!(jobs[0].error.is_none());
    }
}
