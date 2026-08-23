use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("pty error: {0}")]
    Pty(String),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("core error: {0}")]
    Core(#[from] rolen_core::CoreError),

    #[error("provider error: {0}")]
    Provider(#[from] rolen_providers::ProviderError),

    #[error("runtime error: {0}")]
    Runtime(#[from] rolen_runtime::error::RuntimeError),

    #[error("cli '{0}' exited with code {1}")]
    ExitCode(String, i32),

    #[error("pty error: {0}")]
    PtyAny(#[from] anyhow::Error),
}
