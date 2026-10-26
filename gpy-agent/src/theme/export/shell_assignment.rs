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
/// Each of `\`, `$`, `"` and (bash/zsh only; inert in fish) backtick gets one
/// leading backslash, so a trailing `\` can't eat the closing quote.
pub(crate) fn escape_shell_value(text: &str, shell: Shell) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        let needs_escape = match ch {
            '\\' | '$' | '"' => true,
            '`' => matches!(shell, Shell::Bash | Shell::Zsh),
            _ => false,
        };
        if needs_escape {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// Append formatted `args` plus a newline to `output`.
#[expect(clippy::expect_used, reason = "writing to a String is infallible")]
pub(super) fn push_line(output: &mut String, args: std::fmt::Arguments<'_>) {
    use std::fmt::Write;

    writeln!(output, "{args}").expect("string write");
}
