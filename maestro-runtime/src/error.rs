use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("provider error: {0}")]
    Provider(#[from] maestro_providers::ProviderError),

    #[error("core error: {0}")]
    Core(#[from] maestro_core::CoreError),

    #[error("routing error: {0}")]
    Routing(#[from] maestro_core::rules::RuleError),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("sandbox violation: {0}")]
    Sandbox(String),

    #[error("agent did not finish within {0} steps")]
    StepLimit(usize),

    #[error("cancelled")]
    Cancelled,
}
