//! Typed error hierarchy (spec §6.1): `Api`, `Config`, `Auth`, `Io`,
//! `Validation`, and `Aborted` categories, rendered as a single English
//! message on stderr with exit code 1.

use std::io;

/// API failure categories normalized from HTTP status codes (spec §2.1):
/// 400 -> `InvalidArgument`, 401 -> `NotAuthenticated`, 403 -> `NotAuthorized`,
/// 404 -> `NotFound`, everything else -> `UnexpectedApiError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiErrorKind {
    InvalidArgument,
    NotAuthenticated,
    NotAuthorized,
    NotFound,
    UnexpectedApiError,
}

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
