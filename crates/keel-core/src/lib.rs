pub mod agent;
pub mod apps;
pub mod compaction;
pub mod guard;
pub mod hardware;
pub mod harnesses;
pub mod mcp;
pub mod models;
pub mod runtime;
pub mod sessions;
pub mod sql_sandbox;
pub mod tools;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub mod images;

pub mod context;
pub mod documents;
pub mod projects;
pub mod setup;

pub mod recording;
pub mod streaming;

pub mod publish;
pub mod worktrees;
