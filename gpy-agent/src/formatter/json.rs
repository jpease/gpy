//! JSON output formatter

use crate::Result;
use crate::formatter::{Formatter, RenderContext};
use crate::ipc::Response;

/// Serializes responses to JSON.
pub struct JsonFormatter;

impl Formatter for JsonFormatter {
    fn render(&self, response: &Response, _ctx: &RenderContext<'_>) -> Result<String> {
        crate::ipc::wire::encode_json(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::formatter::SegmentPosition;
    use crate::ipc::Response;
    use crate::theme::ThemeConfig;

    fn create_test_context() -> (Config, ThemeConfig) {
        let config = Config::default();
        let theme = ThemeConfig::default();
        (config, theme)
    }

    /// Looks up `pointer` in `value`, panicking with the pointer when absent.
    fn field(value: &serde_json::Value, pointer: &str) -> serde_json::Value {
        value
            .pointer(pointer)
            .cloned()
            .unwrap_or_else(|| panic!("missing JSON field {pointer}"))
    }

    #[test]
    fn json_formatter_emits_non_default_repository_fields() {
        let formatter = JsonFormatter;
        let (config, theme) = create_test_context();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        let response = Response::RepositoryStatus(crate::git::RepositoryStatus {
            branch: "feature/thing".to_owned(),
            ahead: 7_u32,
            behind: 9_u32,
            ahead_capped: true,
            behind_capped: true,
            staged: 1_u32,
            unstaged: 2_u32,
            untracked: 3_u32,
            conflicts: 4_u32,
            state: crate::git::RepositoryState::Rebasing,
            stash_count: 6_u32,
            detached: true,
            rebase_progress: Some(crate::git::RebaseProgress {
                step: 2_u32,
                total: 5_u32,
            }),
        });

        let output = formatter.render(&response, &ctx).expect("render failed");
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("invalid JSON output");

        assert_eq!(field(&parsed, "/branch"), "feature/thing");
        assert_eq!(field(&parsed, "/ahead"), 7_i32);
        assert_eq!(field(&parsed, "/behind"), 9_i32);
        assert_eq!(field(&parsed, "/ahead_capped"), true);
        assert_eq!(field(&parsed, "/behind_capped"), true);
        assert_eq!(field(&parsed, "/staged"), 1_i32);
        assert_eq!(field(&parsed, "/unstaged"), 2_i32);
        assert_eq!(field(&parsed, "/untracked"), 3_i32);
        assert_eq!(field(&parsed, "/conflicts"), 4_i32);
        assert_eq!(field(&parsed, "/state"), "rebasing");
        assert_eq!(field(&parsed, "/stash_count"), 6_i32);
        assert_eq!(field(&parsed, "/detached"), true);
        assert_eq!(field(&parsed, "/rebase_progress/step"), 2_i32);
        assert_eq!(field(&parsed, "/rebase_progress/total"), 5_i32);
    }
}
