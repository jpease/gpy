//! Shell variable assignments and the escaping every exported value goes through.

use crate::shell::Shell;

#[derive(Debug, Clone, Copy)]
enum AssignmentScope {
    Local,
    Export,
    Unset,
}

#[derive(Debug, Clone)]
pub(super) struct ShellAssignment {
    scope: AssignmentScope,
    name: String,
    value: String,
}

impl ShellAssignment {
    pub(super) fn local(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            scope: AssignmentScope::Local,
            name: name.into(),
            value: value.into(),
        }
    }

    pub(super) fn export(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            scope: AssignmentScope::Export,
            name: name.into(),
            value: value.into(),
        }
    }

    pub(super) fn unset(name: impl Into<String>) -> Self {
        Self {
            scope: AssignmentScope::Unset,
            name: name.into(),
            value: String::new(),
        }
    }

    /// Render the assignment, escaping the value once with [`escape_shell_value`].
    fn render(self, shell: Shell) -> String {
        let syntax = shell.variable_syntax();
        match self.scope {
            AssignmentScope::Unset => syntax.format_unset(&self.name),
            AssignmentScope::Local => {
                syntax.format(&self.name, &escape_shell_value(&self.value, shell))
            }
            AssignmentScope::Export => {
                syntax.format_export(&self.name, &escape_shell_value(&self.value, shell))
            }
        }
    }
}

/// Sanitize a free-form theme property key into a shell variable-name fragment.
///
/// Segment names use [`SegmentName::variable_token`](crate::plugin::SegmentName::variable_token)
/// instead: Fish recomputes that token, so it must stay the documented `-`→`_` mapping.
pub(super) fn sanitize_property_token(raw: &str) -> String {
    let mut normalized = String::new();
    let mut prev_was_underscore = false;

    for ch in raw.chars() {
        let is_valid = ch.is_ascii_alphanumeric() || ch == '_';
        let next = if is_valid {
            ch.to_ascii_lowercase()
        } else {
            '_'
        };

        if next == '_' {
            if prev_was_underscore {
                continue;
            }
            prev_was_underscore = true;
        } else {
            prev_was_underscore = false;
        }

        normalized.push(next);
    }

    normalized.trim_matches('_').to_owned()
}

pub(super) fn append_shell_assignment(
    output: &mut String,
    shell: Shell,
    assignment: ShellAssignment,
) {
    output.push_str(&assignment.render(shell));
    output.push('\n');
}

pub(super) fn append_shell_assignments(
    output: &mut String,
    shell: Shell,
    assignments: impl IntoIterator<Item = ShellAssignment>,
) {
    for assignment in assignments {
        append_shell_assignment(output, shell, assignment);
    }
}

/// Escape a theme value for a double-quoted shell assignment.
///
/// Theme values are attacker-influenceable and only checked for control chars upstream.
/// Order matters: `\` first (so later escapes aren't doubled and a trailing `\`
/// can't eat the quote), then `$`, then backtick (bash/zsh only; inert in fish), then `"`.
pub(crate) fn escape_shell_value(text: &str, shell: Shell) -> String {
    let backslash_escaped = text.replace('\\', "\\\\");
    let dollar_escaped = backslash_escaped.replace('$', "\\$");
    let backtick_escaped = match shell {
        Shell::Fish => dollar_escaped,
        Shell::Bash | Shell::Zsh => dollar_escaped.replace('`', "\\`"),
    };
    backtick_escaped.replace('"', "\\\"")
}

/// Append formatted `args` plus a newline to `output`.
#[expect(clippy::expect_used, reason = "writing to a String is infallible")]
pub(super) fn push_line(output: &mut String, args: std::fmt::Arguments<'_>) {
    use std::fmt::Write;

    writeln!(output, "{args}").expect("string write");
}
