//! Refinery migration bundle. The migrations live at the workspace
//! `migrations/` directory; refinery's `embed_migrations!` macro inlines
//! them at compile time so the binary needs no runtime SQL files.

// `embed_migrations!` looks under the path relative to the package root
// (the crate's `Cargo.toml`). The workspace migrations dir is two levels
// up: `crates/kb-store-sqlite/Cargo.toml` → `../../migrations`.
refinery::embed_migrations!("../../migrations");

/// Re-export the runner under a stable name. Calling
/// `runner().run(&mut conn)?` applies all pending migrations.
pub fn runner() -> refinery::Runner {
    migrations::runner()
}
