//! Core primitives shared by commands and services (spec §3): project
//! configuration (`.clasp.json`), the Apps Script manifest
//! (`appsscript.json`), ignore matching (`.claspignore`), path jail
//! validation, and generic pagination.

pub mod apis;
pub mod clasp;
pub mod config;
pub mod files;
pub mod ignore;
pub mod manifest;
pub mod pagination;
pub mod path;
pub mod project;
pub mod services;
