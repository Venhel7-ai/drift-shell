//! On-screen compositor error bar state. Errors are keyed by source so each
//! source contributes at most one message; the bar shows the union and clears
//! per-source as each problem is resolved (config reload, background reload).

use super::DriftWm;

/// Collapse config warnings into one line for the error bar (the bar has room
/// for one; the full list stays in the log): first message, plus `(+N more)`.
pub fn summarize_config_errors(warnings: &[String]) -> Option<String> {
    let first = warnings.first()?;
    Some(match warnings.len() {
        1 => first.clone(),
        n => format!("{first} (+{} more)", n - 1),
    })
}

/// Source of a compositor-generated error shown in the on-screen error bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ErrorSource {
    /// Config file read/parse failure (the old config stays active).
    Config,
    /// Background image load/upload failure (falls back to the default shader).
    Background,
    /// Invalid keyboard layout/variant/options/model (falls back to the default
    /// layout at startup, or keeps the previous layout on reload).
    Keyboard,
}

impl DriftWm {
    /// Show `message` for `source` in the error bar. No-op (and no redraw) when
    /// that source's message is unchanged.
    pub fn set_error(&mut self, source: ErrorSource, message: impl Into<String>) {
        let message = message.into();
        if self.errors.get(&source) != Some(&message) {
            self.errors.insert(source, message);
            self.mark_all_dirty();
        }
    }

    /// Clear `source`'s error once the problem is resolved. No-op if not set.
    pub fn clear_error(&mut self, source: ErrorSource) {
        if self.errors.remove(&source).is_some() {
            self.mark_all_dirty();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::summarize_config_errors;

    #[test]
    fn summarize_empty_is_none() {
        assert_eq!(summarize_config_errors(&[]), None);
    }

    #[test]
    fn summarize_single_is_verbatim() {
        let w = vec!["bad thing".to_string()];
        assert_eq!(summarize_config_errors(&w), Some("bad thing".to_string()));
    }

    #[test]
    fn summarize_multiple_appends_count() {
        let w = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(summarize_config_errors(&w), Some("a (+2 more)".to_string()));
    }
}
