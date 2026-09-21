//! Minimal unified-style diff for Edit's approval card. Per aldwin-tools.md:
//! "The diff is rendered from the (before, after) the tool already assembled
//! — TUI owns formatting" — so this only needs to produce the +/- line
//! content; syntax highlighting and layout are the TUI's job. `before`/
//! `after` are a single replaced hunk (not a whole file), so a plain O(n*m)
//! LCS line diff is more than fast enough.

enum DiffOp<'a> {
    Context(&'a str),
    Removed(&'a str),
    Added(&'a str),
}

/// Renders `before` -> `after` as a unified diff of `path`.
pub fn unified(path: &str, before: &str, after: &str) -> String {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();

    let mut out = format!("--- {path}\n+++ {path}\n");
    for op in diff_lines(&before_lines, &after_lines) {
        match op {
            DiffOp::Context(line) => out.push_str(&format!(" {line}\n")),
            DiffOp::Removed(line) => out.push_str(&format!("-{line}\n")),
            DiffOp::Added(line) => out.push_str(&format!("+{line}\n")),
        }
    }
    out
}

fn diff_lines<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<DiffOp<'a>> {
    let (n, m) = (a.len(), b.len());

    // lcs_len[i][j] = length of the LCS of a[i..] and b[j..].
    let mut lcs_len = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs_len[i][j] =
                if a[i] == b[j] { lcs_len[i + 1][j + 1] + 1 } else { lcs_len[i + 1][j].max(lcs_len[i][j + 1]) };
        }
    }

    let mut ops = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push(DiffOp::Context(a[i]));
            i += 1;
            j += 1;
        } else if lcs_len[i + 1][j] >= lcs_len[i][j + 1] {
            ops.push(DiffOp::Removed(a[i]));
            i += 1;
        } else {
            ops.push(DiffOp::Added(b[j]));
            j += 1;
        }
    }
    while i < n {
        ops.push(DiffOp::Removed(a[i]));
        i += 1;
    }
    while j < m {
        ops.push(DiffOp::Added(b[j]));
        j += 1;
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_is_all_context() {
        let out = unified("f.rs", "a\nb\n", "a\nb\n");
        assert_eq!(out, "--- f.rs\n+++ f.rs\n a\n b\n");
    }

    #[test]
    fn pure_addition_has_no_removed_lines() {
        let out = unified("f.rs", "a\n", "a\nb\n");
        let body: Vec<&str> = out.lines().skip(2).collect(); // drop --- / +++ headers
        assert!(body.iter().all(|l| !l.starts_with('-')));
        assert!(body.contains(&"+b"));
    }

    #[test]
    fn single_line_replace_shows_remove_then_add() {
        let out = unified("f.rs", "old\n", "new\n");
        assert!(out.contains("-old"));
        assert!(out.contains("+new"));
    }

    #[test]
    fn shared_context_around_a_change_is_preserved() {
        let out = unified("f.rs", "a\nold\nc\n", "a\nnew\nc\n");
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines.contains(&" a"));
        assert!(lines.contains(&"-old"));
        assert!(lines.contains(&"+new"));
        assert!(lines.contains(&" c"));
    }
}
