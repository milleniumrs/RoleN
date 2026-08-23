use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("could not determine the platform config directory")]
    NoConfigDir,

    #[error("could not determine the platform data directory")]
    NoDataDir,

    #[error("could not determine the home directory")]
    NoHomeDir,

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("keychain error: {0}")]
    Keyring(#[from] keyring::Error),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("vault is locked: set the ROLEN_VAULT_PASSWORD environment variable")]
    VaultLocked,

    #[error("vault error: {0}")]
    Vault(String),
}
