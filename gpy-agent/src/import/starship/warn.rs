//! Collects non-fatal import warnings and renders them grouped for the CLI.

/// The category of an import warning, used to group the rendered output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    /// A Starship module has no GPY equivalent and was skipped.
    UnsupportedModule,
    /// A Starship option could not be represented in a GPY artifact.
    UnrepresentableOption,
    /// A color value failed validation and was dropped.
    InvalidColor,
    /// A construct mapped only approximately (information was lost).
    LossyMapping,
}

/// A single non-fatal import warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// The warning category.
    pub kind: WarningKind,
    /// The human-readable detail.
    pub message: String,
}

/// An ordered, groupable collection of import warnings.
#[derive(Debug, Clone, Default)]
pub struct Warnings {
    items: Vec<Warning>,
}

impl Warnings {
    /// Create an empty collector.
    #[must_use]
    pub const fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Record a warning.
    pub fn push(&mut self, kind: WarningKind, message: impl Into<String>) {
        self.items.push(Warning {
            kind,
            message: message.into(),
        });
    }

    /// Whether no warnings were recorded.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Number of recorded warnings.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.items.len()
    }

    /// Iterate the recorded warnings in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Warning> {
        self.items.iter()
    }

    /// Render the warnings grouped by category, in a deterministic group order.
    #[must_use]
    pub fn render(&self) -> String {
        const ORDER: [(WarningKind, &str); 4_usize] = [
            (WarningKind::UnsupportedModule, "Unsupported modules"),
            (
                WarningKind::UnrepresentableOption,
                "Unrepresentable options",
            ),
            (WarningKind::InvalidColor, "Invalid colors"),
            (WarningKind::LossyMapping, "Lossy mappings"),
        ];
        let mut out = String::new();
        for (kind, header) in ORDER {
            let group: Vec<&Warning> = self.items.iter().filter(|w| w.kind == kind).collect();
            if group.is_empty() {
                continue;
            }
            out.push_str(header);
            out.push_str(":\n");
            for warning in group {
                out.push_str("  - ");
                out.push_str(&warning.message);
                out.push('\n');
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{WarningKind, Warnings};

    #[test]
    fn empty_collector_renders_nothing() {
        let warnings = Warnings::new();
        assert!(warnings.is_empty());
        assert_eq!(warnings.len(), 0_usize);
        assert_eq!(warnings.render(), "");
    }

    #[test]
    fn groups_by_kind_in_deterministic_order() {
        let mut warnings = Warnings::new();
        warnings.push(WarningKind::InvalidColor, "bad color zzz");
        warnings.push(WarningKind::UnsupportedModule, "module aws skipped");
        warnings.push(WarningKind::UnsupportedModule, "module battery skipped");
        let rendered = warnings.render();
        let unsupported = rendered.find("Unsupported modules").expect("group present");
        let invalid = rendered.find("Invalid colors").expect("group present");
        assert!(
            unsupported < invalid,
            "unsupported group must precede invalid group"
        );
        assert!(rendered.contains("  - module aws skipped"));
        assert!(rendered.contains("  - module battery skipped"));
        assert_eq!(warnings.len(), 3_usize);
    }
}
