//! `gpy prompt` against an agent that is down, wedged, scripted or real
//! (#1249): the instant cache's stale, chunked-key and variant-fallback
//! paths, registering again after the agent lost the registration, the
//! late window a never-answering agent opens, the colour depth of the
//! agent's segments (#1265), and warnings shown once per shell.
//! The shell end-to-end tests that covered these went with the shell
//! renderers (#1247). Socket-dependent, so Unix-only.
#![cfg(unix)]
#![allow(clippy::unwrap_used, reason = "test setup fails loudly")]
#![allow(clippy::expect_used, reason = "test setup fails loudly")]
#![allow(clippy::panic, reason = "assertion helpers panic with context")]

#[path = "../common/mod.rs"]
mod common;

mod cache;
mod depth;
mod harness;
mod late;
mod registration;
mod warnings;
