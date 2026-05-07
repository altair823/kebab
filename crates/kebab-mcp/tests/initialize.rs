//! Integration: KebabHandler::get_info returns correct kebab serverInfo.
//! Doesn't exercise full transport — that lands when we have at least
//! one tool to call (Task 4+).

use kebab_config::Config;
use kebab_mcp::{KebabAppState, KebabHandler};
use rmcp::ServerHandler;

#[tokio::test]
async fn initialize_returns_kebab_server_info() {
    let cfg = Config::defaults();
    let state = KebabAppState::new(cfg);
    let handler = KebabHandler::new(state);

    let info = handler.get_info();
    assert_eq!(info.server_info.name, "kebab");
    assert!(!info.server_info.version.is_empty());
    assert!(info.capabilities.tools.is_some());
}
