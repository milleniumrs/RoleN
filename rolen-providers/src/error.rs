use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("core error: {0}")]
    Core(#[from] rolen_core::CoreError),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("provider not found: {0}")]
    NotFound(String),

    #[error("provider '{0}' has no endpoint configured")]
    NoEndpoint(String),

    #[error("api error: {0}")]
    Api(String),

    #[error("unexpected response shape: {0}")]
    Parse(String),
}
