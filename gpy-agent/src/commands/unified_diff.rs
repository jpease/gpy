//! Line diff in unified format, for `gpy theme diff`.
//!
//! Theme files are a few hundred lines, so a longest-common-subsequence table
//! over the lines between the common prefix and suffix is cheap. Inputs whose
//! table would be unreasonably large degrade to "every line replaced" rather
//! than allocating it.

/// Unchanged lines shown around each change.
const CONTEXT: usize = 3;

/// Largest LCS table, in cells, that is worth allocating (16 MB of `u32`).
const MAX_TABLE_CELLS: usize = 4_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Keep,
    Delete,
    Insert,
}

/// One diff line with the counts of old and new lines that precede it.
struct Op<'a> {
    kind: Kind,
    text: &'a str,
    old_before: usize,
    new_before: usize,
}

/// A unified diff turning `old` into `new`, or `None` when their lines are equal.
#[must_use]
pub fn unified(old_label: &str, new_label: &str, old: &str, new: &str) -> Option<String> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let ops = number_ops(diff_lines(&old_lines, &new_lines));
    if ops.iter().all(|op| op.kind == Kind::Keep) {
        return None;
    }

    let mut out = format!("--- {old_label}\n+++ {new_label}\n");
    for (start, end) in hunk_ranges(&ops) {
        write_hunk(
            &mut out,
            ops.iter().skip(start).take(end.saturating_sub(start)),
        );
    }
    Some(out)
}

/// Edit script from `old` to `new`, deletions before insertions within a change.
fn diff_lines<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(Kind, &'a str)> {
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(old.len().min(new.len()).saturating_sub(prefix))
        .take_while(|(left, right)| left == right)
        .count();
    let old_mid = old
        .iter()
        .skip(prefix)
        .take(old.len().saturating_sub(prefix).saturating_sub(suffix))
        .copied()
        .collect::<Vec<_>>();
    let new_mid = new
        .iter()
        .skip(prefix)
        .take(new.len().saturating_sub(prefix).saturating_sub(suffix))
        .copied()
        .collect::<Vec<_>>();

    let keep = |lines: &[&'a str], count: usize, skip: usize| {
        lines
            .iter()
            .skip(skip)
            .take(count)
            .map(|line| (Kind::Keep, *line))
            .collect::<Vec<_>>()
    };
    let mut ops = keep(old, prefix, 0);
    ops.extend(diff_middle(&old_mid, &new_mid));
    ops.extend(keep(old, suffix, old.len().saturating_sub(suffix)));
    ops
}

/// LCS-based edit script for the lines that differ between prefix and suffix.
#[allow(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "every index is bounded by the table dimensions (rows * width cells) and the two line counts the loops run over"
)]
fn diff_middle<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(Kind, &'a str)> {
    let (rows, cols) = (old.len(), new.len());
    let width = cols + 1;
    if rows == 0 || cols == 0 || (rows + 1).saturating_mul(width) > MAX_TABLE_CELLS {
        let deleted = old.iter().map(|line| (Kind::Delete, *line));
        let inserted = new.iter().map(|line| (Kind::Insert, *line));
        return deleted.chain(inserted).collect();
    }

    // table[i * width + j] = length of the LCS of old[i..] and new[j..].
    let mut table = vec![0_u32; (rows + 1) * width];
    for i in (0..rows).rev() {
        for j in (0..cols).rev() {
            table[i * width + j] = if old[i] == new[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }

    let (mut i, mut j) = (0, 0);
    let mut ops = Vec::with_capacity(rows + cols);
    while i < rows && j < cols {
        if old[i] == new[j] {
            ops.push((Kind::Keep, old[i]));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            ops.push((Kind::Delete, old[i]));
            i += 1;
        } else {
            ops.push((Kind::Insert, new[j]));
            j += 1;
        }
    }
    ops.extend(old.iter().skip(i).map(|line| (Kind::Delete, *line)));
    ops.extend(new.iter().skip(j).map(|line| (Kind::Insert, *line)));
    ops
}

/// Attach the running old/new line counts to each op.
fn number_ops(script: Vec<(Kind, &str)>) -> Vec<Op<'_>> {
    let (mut old_before, mut new_before) = (0_usize, 0_usize);
    script
        .into_iter()
        .map(|(kind, text)| {
            let op = Op {
                kind,
                text,
                old_before,
                new_before,
            };
            if kind != Kind::Insert {
                old_before = old_before.saturating_add(1);
            }
            if kind != Kind::Delete {
                new_before = new_before.saturating_add(1);
            }
            op
        })
        .collect()
}

/// `[start, end)` op ranges, one per hunk: each change plus [`CONTEXT`] lines
/// either side, merged where the unchanged gap between changes is short.
fn hunk_ranges(ops: &[Op<'_>]) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for index in ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op.kind != Kind::Keep)
        .map(|(index, _)| index)
    {
        let start = index.saturating_sub(CONTEXT);
        let end = index
            .saturating_add(1)
            .saturating_add(CONTEXT)
            .min(ops.len());
        match ranges.last_mut() {
            Some(last) if start <= last.1 => last.1 = end,
            _ => ranges.push((start, end)),
        }
    }
    ranges
}

fn write_hunk<'a>(out: &mut String, ops: impl Iterator<Item = &'a Op<'a>> + Clone) {
    let old_len = ops.clone().filter(|op| op.kind != Kind::Insert).count();
    let new_len = ops.clone().filter(|op| op.kind != Kind::Delete).count();
    let (old_before, new_before) = ops
        .clone()
        .next()
        .map_or((0, 0), |op| (op.old_before, op.new_before));
    // A side with no lines in the hunk is named by the line before it.
    let first = |before: usize, len: usize| {
        if len == 0 {
            before
        } else {
            before.saturating_add(1)
        }
    };
    let header = format!(
        "@@ -{},{old_len} +{},{new_len} @@\n",
        first(old_before, old_len),
        first(new_before, new_len)
    );
    out.push_str(&header);
    for op in ops {
        let marker = match op.kind {
            Kind::Keep => ' ',
            Kind::Delete => '-',
            Kind::Insert => '+',
        };
        out.push(marker);
        out.push_str(op.text);
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::unified;

    #[test]
    fn identical_text_has_no_diff() {
        assert_eq!(unified("a", "b", "x\ny\n", "x\ny\n"), None);
    }

    #[test]
    fn a_changed_line_shows_with_context_and_line_numbers() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let new = "1\n2\n3\n4\nFIVE\n6\n7\n8\n9\n";
        let diff = unified("old", "new", old, new).expect("differs");
        assert_eq!(
            diff,
            "--- old\n+++ new\n@@ -2,7 +2,7 @@\n 2\n 3\n 4\n-5\n+FIVE\n 6\n 7\n 8\n"
        );
    }

    #[test]
    fn distant_changes_make_separate_hunks_and_near_ones_merge() {
        let old: String = (1_u32..=30).fold(String::new(), |mut acc, n| {
            acc.push_str(&n.to_string());
            acc.push('\n');
            acc
        });
        let far = old
            .replacen("\n3\n", "\nthree\n", 1)
            .replacen("\n28\n", "\ntwenty-eight\n", 1);
        assert_eq!(
            unified("a", "b", &old, &far)
                .expect("differs")
                .matches("@@ -")
                .count(),
            2
        );
        let near = old
            .replacen("\n3\n", "\nthree\n", 1)
            .replacen("\n7\n", "\nseven\n", 1);
        assert_eq!(
            unified("a", "b", &old, &near)
                .expect("differs")
                .matches("@@ -")
                .count(),
            1
        );
    }

    #[test]
    fn pure_insertion_and_deletion_name_the_preceding_line() {
        let inserted = unified("a", "b", "x\n", "x\ny\n").expect("differs");
        assert_eq!(inserted, "--- a\n+++ b\n@@ -1,1 +1,2 @@\n x\n+y\n");
        let emptied = unified("a", "b", "x\n", "").expect("differs");
        assert_eq!(emptied, "--- a\n+++ b\n@@ -1,1 +0,0 @@\n-x\n");
    }

    #[test]
    fn moved_blocks_are_reported_as_delete_and_insert() {
        let diff = unified("a", "b", "a\nb\nc\nd\n", "a\nd\nb\nc\n").expect("differs");
        assert!(diff.contains("-d\n") && diff.contains("+d\n"), "{diff}");
    }
}
