pub mod agent;
pub mod apps;
pub mod audio;
pub mod carousel;
pub mod compaction;
pub mod decide;
pub mod eval_cases;
pub mod guard;
pub mod hardware;
pub mod harnesses;
pub mod mcp;
pub mod model_check;
pub mod models;
pub mod packs;
pub mod remote;
pub mod runtime;
pub mod sessions;
pub mod sql_sandbox;
pub mod tools;
pub mod web_search;

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
pub mod updates;
pub mod video;
pub mod worktrees;
