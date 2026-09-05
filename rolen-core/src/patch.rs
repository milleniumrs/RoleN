//! Unified-diff patch application (PRD FR-7.9).
//!
//! A patch ticket carries a unified diff instead of full content. The diff is
//! applied to the *current* file content: hunks are first tried at their
//! declared position, then searched for at other offsets (the "3-way merge
//! attempt" — disjoint concurrent edits to the same file still merge). A hunk
//! that matches nowhere, or matches ambiguously at several positions, rejects
//! the whole ticket — nothing is half-applied.

/// Apply a unified diff to `old`, returning the new content, or None when the
/// patch does not apply cleanly and unambiguously.
pub fn apply_patch(old: &str, diff: &str) -> Option<String> {
    let hunks = parse_hunks(diff)?;
    if hunks.is_empty() {
        return None;
    }
    let old_lines: Vec<&str> = old.split('\n').collect();
    // split() on a trailing newline leaves a phantom empty final element
    let mut lines: Vec<String> = old_lines
        .iter()
        .map(|s| s.strip_suffix('\r').unwrap_or(s).to_string())
        .collect();
    let trailing_newline = old.ends_with('\n') || old.is_empty();
    if trailing_newline && lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }

    // apply hunks bottom-up so earlier splices don't shift later positions
    let mut ordered = hunks;
    ordered.sort_by_key(|h| std::cmp::Reverse(h.old_start));
    for h in &ordered {
        let pos = locate(&lines, h)?;
        lines.splice(pos..pos + h.old_len(), h.new_lines());
    }

    let mut out = lines.join("\n");
    if trailing_newline {
        out.push('\n');
    }
    Some(out)
}

struct Hunk {
    /// declared 1-based start line in the old file (from `@@ -start,len @@`)
    old_start: usize,
    /// context + removed lines: what must match the old content
    old_side: Vec<String>,
    /// context + added lines: what replaces it
    new_side: Vec<String>,
}

impl Hunk {
    fn old_len(&self) -> usize {
        self.old_side.len()
    }
    fn new_lines(&self) -> Vec<String> {
        self.new_side.clone()
    }
}

fn parse_hunks(diff: &str) -> Option<Vec<Hunk>> {
    let mut hunks = Vec::new();
    let mut cur: Option<Hunk> = None;
    for line in diff.lines() {
        if line.starts_with("---") || line.starts_with("+++") || line.starts_with("diff ") {
            continue;
        }
        if line.starts_with("@@") {
            if let Some(h) = cur.take() {
                hunks.push(h);
            }
            // @@ -old_start[,old_len] +new_start[,new_len] @@
            let old_start = line
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.trim_start_matches('-').split(',').next())
                .and_then(|s| s.parse::<usize>().ok())?;
            cur = Some(Hunk {
                old_start,
                old_side: Vec::new(),
                new_side: Vec::new(),
            });
            continue;
        }
        let Some(h) = cur.as_mut() else {
            // content before the first hunk header — not a unified diff
            return None;
        };
        match line.as_bytes().first() {
            Some(b' ') => {
                let ctx = line[1..].to_string();
                h.old_side.push(ctx.clone());
                h.new_side.push(ctx);
            }
            Some(b'-') => h.old_side.push(line[1..].to_string()),
            Some(b'+') => h.new_side.push(line[1..].to_string()),
            // "\ No newline at end of file" and empty lines (diff-empty ctx)
            _ => {
                if line.is_empty() {
                    h.old_side.push(String::new());
                    h.new_side.push(String::new());
                }
            }
        }
    }
    if let Some(h) = cur.take() {
        hunks.push(h);
    }
    Some(hunks)
}

/// Find where a hunk's old side matches `lines`: first at the declared
/// position, then at every other offset (fuzzy 3-way). Returns the unique
/// match position; None when zero or several positions match.
fn locate(lines: &[String], h: &Hunk) -> Option<usize> {
    let needle: Vec<&str> = h.old_side.iter().map(|s| s.as_str()).collect();
    if needle.is_empty() {
        // pure-addition hunk onto an (empty or any) file: insert at the
        // declared position, clamped to the file end
        return Some(h.old_start.saturating_sub(1).min(lines.len()));
    }
    let declared = h.old_start.saturating_sub(1);
    let mut matches = Vec::new();
    for start in 0..=lines.len().saturating_sub(needle.len()) {
        if lines[start..start + needle.len()]
            .iter()
            .map(|s| s.as_str())
            .eq(needle.iter().copied())
        {
            matches.push(start);
        }
    }
    if matches.len() > 1 {
        // prefer the declared position when it is among the matches
        return if matches.contains(&declared) {
            Some(declared)
        } else {
            None // ambiguous — reject rather than guess
        };
    }
    matches.first().copied()
}

/// Minimal unified-style line diff (LCS), for diff previews before
/// regenerating files like AGENTS.md (FR-5.3). Not a git-grade diff — good
/// enough for a human to review what would change.
pub fn simple_diff(old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // LCS table
    let mut dp = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = String::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            out.push_str(&format!("  {}\n", a[i]));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push_str(&format!("- {}\n", a[i]));
            i += 1;
        } else {
            out.push_str(&format!("+ {}\n", b[j]));
            j += 1;
        }
    }
    for line in &a[i..] {
        out.push_str(&format!("- {line}\n"));
    }
    for line in &b[j..] {
        out.push_str(&format!("+ {line}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_marks_changes() {
        let d = simple_diff("a\nb\nc\n", "a\nB\nc\n");
        assert!(d.contains("- b"));
        assert!(d.contains("+ B"));
        assert!(d.contains("  a"));
    }

    #[test]
    fn applies_at_declared_position() {
        let old = "a\nb\nc\n";
        let diff = "@@ -2,1 +2,1 @@\n-b\n+B\n";
        assert_eq!(apply_patch(old, diff).unwrap(), "a\nB\nc\n");
    }

    #[test]
    fn fuzzy_offset_merges_disjoint_concurrent_edit() {
        // base the patch was written against
        // (someone else inserted two lines at the top meanwhile)
        let old = "INSERTED1\nINSERTED2\na\nb\nc\n";
        let diff = "@@ -2,3 +2,3 @@\n a\n-b\n+B\n c\n";
        assert_eq!(
            apply_patch(old, diff).unwrap(),
            "INSERTED1\nINSERTED2\na\nB\nc\n"
        );
    }

    #[test]
    fn rejects_when_context_gone() {
        let old = "x\ny\nz\n";
        let diff = "@@ -2,1 +2,1 @@\n-b\n+B\n";
        assert!(apply_patch(old, diff).is_none());
    }

    #[test]
    fn rejects_ambiguous_match() {
        // "a\nb" occurs at lines 1 and 3, and the declared position (2) no
        // longer matches — too dangerous to guess
        let old = "a\nb\na\nb\n";
        let diff = "@@ -2,2 +2,2 @@\n-a\n-b\n+A\n+B\n";
        assert!(apply_patch(old, diff).is_none());
    }

    #[test]
    fn prefers_declared_position_when_ambiguous() {
        let old = "a\nb\na\nb\n";
        // "a\nb" occurs at lines 1 and 3; the declared position (1) wins
        let diff = "@@ -1,2 +1,2 @@\n-a\n-b\n+A\n+B\n";
        assert_eq!(apply_patch(old, diff).unwrap(), "A\nB\na\nb\n");
    }

    #[test]
    fn creates_file_from_pure_addition() {
        let diff = "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+hello\n+world\n";
        assert_eq!(apply_patch("", diff).unwrap(), "hello\nworld\n");
    }

    #[test]
    fn rejects_garbage() {
        assert!(apply_patch("x", "not a diff").is_none());
        assert!(apply_patch("x", "").is_none());
    }

    #[test]
    fn multiple_hunks_apply_bottom_up() {
        let old = "1\n2\n3\n4\n5\n";
        let diff = "@@ -1,1 +1,1 @@\n-1\n+one\n@@ -5,1 +5,1 @@\n-5\n+five\n";
        assert_eq!(apply_patch(old, diff).unwrap(), "one\n2\n3\n4\nfive\n");
    }
}
