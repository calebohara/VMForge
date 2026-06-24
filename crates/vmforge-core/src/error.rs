//! Crate-wide error type. Every engine fallible operation returns
//! [`Result<T>`]. IPC commands convert this to a `String` at the boundary.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("QEMU binary not found: {0}")]
    QemuNotFound(String),

    #[error("process error: {0}")]
    Process(String),

    #[error("QMP error: {0}")]
    Qmp(String),

    #[error("VM not found: {0}")]
    VmNotFound(String),

    #[error("invalid configuration: {0}")]
    Config(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("not implemented: {0}")]
    NotImplemented(&'static str),

    #[error("{0}")]
    Other(String),
}
