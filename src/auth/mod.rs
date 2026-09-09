//! Authentication (spec §2.3 `.clasprc.json`, §2.4 OAuth flows, §7.4 token
//! refresh): file credential store with clasp V1/V3 compatibility, the
//! default/user-provided OAuth client, PKCE authorization-code flows
//! (localhost + serverless), and the login/logout/show-authorized-user
//! services.

pub mod credential_store;
pub mod flow;
pub mod localhost_flow;
pub mod oauth_client;
pub mod serverless_flow;

pub use crate::auth::credential_store::{CredentialStore, StoredCredentials};
pub use crate::auth::flow::{
    AuthOptions, LoginPayload, LogoutResult, load_credentials, login, logout, refresh_and_save,
    show_authorized_user,
};
