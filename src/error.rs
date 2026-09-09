//! Typed error hierarchy (spec §6.1): `Api`, `Config`, `Auth`, `Io`,
//! `Validation`, and `Aborted` categories, rendered as a single English
//! message on stderr with exit code 1.

use std::io;

pub use crate::api::error::ApiErrorKind;

/// The single error type surfaced by [`crate::run`] and rendered by `main`.
#[derive(Debug, thiserror::Error)]
pub enum CrspError {
    #[error("API error ({kind:?}): {message}")]
    Api { kind: ApiErrorKind, message: String },
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Auth(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("{0}")]
    Validation(String),
    #[error("Aborted.")]
    Aborted,
    #[error("{0} is not implemented yet.")]
    NotImplemented(String),
}
