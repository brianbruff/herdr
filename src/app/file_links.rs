//! Clickable file references in pane output.
//!
//! Coding agents cite code as plain text (`intent.py:303`,
//! `src/app/mod.rs:22:5`, `lib/x.py:69-73`). This module finds the reference
//! under a clicked cell, resolves it to a file in the pane's worktree, and
//! expands the user's `[editor] open_file` command template for it.

use std::path::{Component, Path, PathBuf};

/// Upper bound on suffix-search candidates offered to the picker.
const MAX_SUFFIX_CANDIDATES: usize = 200;

/// Recent pane lines scanned to disambiguate a bare file name.
pub(crate) const SCROLLBACK_HINT_LINES: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileRef {
    pub(crate) path: String,
    pub(crate) line: Option<u32>,
    pub(crate) column: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolution {
    None,
    One(PathBuf),
    Many(Vec<PathBuf>),
}

/// Finds the file reference in `line` that covers character index `char_idx`.
pub(crate) fn file_ref_at_char(line: &str, char_idx: usize) -> Option<FileRef> {
    let chars: Vec<char> = line.chars().collect();
    if chars.get(char_idx).is_none_or(|ch| ch.is_whitespace()) {
        return None;
    }

    let mut start = char_idx;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = char_idx + 1;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }

    while start < end && is_leading_wrapper(chars[start]) {
        start += 1;
    }
    while end > start && is_trailing_wrapper(chars[end - 1]) {
        end -= 1;
    }
    if !(start..end).contains(&char_idx) {
        return None;
    }

    let token: String = chars[start..end].iter().collect();
    parse_file_ref(&token)
}

/// Parses one already-trimmed token, or a `file://` hyperlink URI.
pub(crate) fn parse_file_ref(token: &str) -> Option<FileRef> {
    if let Some(rest) = token.strip_prefix("file://") {
        let path = rest.find('/').map(|idx| &rest[idx..])?;
        let path = percent_decode(path)?;
        return parse_path_and_position(&path).or(Some(FileRef {
            path,
            line: None,
            column: None,
        }));
    }
    if token.contains("://") {
        return None;
    }

    if let Some(file_ref) = parse_path_and_position(token) {
        return Some(file_ref);
    }

    // A bare token only counts when it looks like a path.
    (token.contains('/') && is_plausible_path(token)).then(|| FileRef {
        path: token.to_string(),
        line: None,
        column: None,
    })
}

fn parse_path_and_position(token: &str) -> Option<FileRef> {
    let (path, line, column) =
        split_github_anchor(token).or_else(|| split_colon_position(token))?;
    if !is_plausible_path(path) {
        return None;
    }
    Some(FileRef {
        path: path.to_string(),
        line: Some(line),
        column,
    })
}

/// `path#L10`, `path#L10C2`, and ranges like `path#L10-L20`.
fn split_github_anchor(token: &str) -> Option<(&str, u32, Option<u32>)> {
    let (path, anchor) = token.rsplit_once("#L")?;
    let start = anchor.split_once('-').map_or(anchor, |(start, _)| start);
    let (line, column) = match start.split_once('C') {
        Some((line, column)) => (line, Some(parse_number(column)?)),
        None => (start, None),
    };
    Some((path, parse_number(line)?, column))
}

/// `path:line`, `path:line:col`, and ranges like `path:69-73` or
/// `path:1:2-3:4`; ranges open at their start.
fn split_colon_position(token: &str) -> Option<(&str, u32, Option<u32>)> {
    let position = match token.rsplit_once('-') {
        Some((head, tail))
            if head.ends_with(|ch: char| ch.is_ascii_digit()) && is_range_end(tail) =>
        {
            head
        }
        _ => token,
    };
    let (head, last) = position.rsplit_once(':')?;
    let last = parse_number(last)?;
    match head.rsplit_once(':') {
        Some((path, line)) if is_digits(line) => Some((path, parse_number(line)?, Some(last))),
        _ => Some((head, last, None)),
    }
}

fn is_range_end(tail: &str) -> bool {
    match tail.split_once(':') {
        Some((line, column)) => is_digits(line) && is_digits(column),
        None => is_digits(tail),
    }
}

fn is_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn parse_number(value: &str) -> Option<u32> {
    is_digits(value).then(|| value.parse().ok()).flatten()
}

fn is_plausible_path(path: &str) -> bool {
    !path.is_empty()
        && !path.ends_with('/')
        && !path.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
        && path.chars().any(|ch| ch.is_alphanumeric())
}

fn is_leading_wrapper(ch: char) -> bool {
    matches!(ch, '(' | '[' | '{' | '<' | '"' | '\'' | '`')
}

fn is_trailing_wrapper(ch: char) -> bool {
    matches!(
        ch,
        ')' | ']' | '}' | '>' | '"' | '\'' | '`' | '.' | ',' | ';' | ':' | '!' | '?'
    )
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] == b'%' {
            let hex = value.get(idx + 1..idx + 3)?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            idx += 3;
        } else {
            decoded.push(bytes[idx]);
            idx += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// Resolves a reference without searching: absolute, then pane cwd, then root.
pub(crate) fn direct_candidate(
    raw: &str,
    cwd: &Path,
    root: &Path,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let expanded = match (raw.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(raw),
    };
    if expanded.is_absolute() {
        return expanded.is_file().then_some(expanded);
    }
    [cwd.join(&expanded), root.join(&expanded)]
        .into_iter()
        .find(|candidate| candidate.is_file())
}

/// Builds the git pathspec that matches `raw` as a path suffix inside a repo.
pub(crate) fn suffix_pathspec(raw: &str) -> Option<String> {
    let raw = raw.strip_prefix("./").unwrap_or(raw);
    let path = Path::new(raw);
    let plain = path
        .components()
        .all(|component| matches!(component, Component::Normal(_)));
    if raw.is_empty()
        || !plain
        || raw
            .chars()
            .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '\\' | ':'))
    {
        return None;
    }
    Some(format!(":(glob)**/{raw}"))
}

/// Lists tracked and untracked-but-not-ignored files under `root` whose path
/// ends with `raw`.
pub(crate) fn suffix_candidates(root: &Path, raw: &str) -> Vec<PathBuf> {
    let Some(pathspec) = suffix_pathspec(raw) else {
        return Vec::new();
    };
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .arg(pathspec)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let mut candidates: Vec<PathBuf> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|entry| std::str::from_utf8(entry).ok())
        .filter(|entry| !entry.is_empty())
        .map(|entry| root.join(entry))
        .filter(|candidate| candidate.is_file())
        .collect();
    candidates.sort();
    candidates.dedup();
    candidates.truncate(MAX_SUFFIX_CANDIDATES);
    candidates
}

pub(crate) fn resolve(
    raw: &str,
    cwd: &Path,
    root: &Path,
    home: Option<&Path>,
    search: impl FnOnce(&Path, &str) -> Vec<PathBuf>,
) -> Resolution {
    if let Some(path) = direct_candidate(raw, cwd, root, home) {
        return Resolution::One(path);
    }
    if Path::new(raw).is_absolute() || raw.starts_with("~/") {
        return Resolution::None;
    }
    let mut candidates = search(root, raw);
    match candidates.len() {
        0 => Resolution::None,
        1 => Resolution::One(candidates.remove(0)),
        _ => Resolution::Many(candidates),
    }
}

/// Picks the single candidate whose repo-relative path the pane already
/// printed in full, such as an earlier `Read power_analyst/x/intent.py`.
pub(crate) fn candidate_named_in_text(
    candidates: &[PathBuf],
    root: &Path,
    text: &str,
) -> Option<PathBuf> {
    let mut named = candidates.iter().filter(|candidate| {
        relative_label(candidate, root).is_some_and(|rel| contains_path(text, &rel))
    });
    let first = named.next()?;
    named.next().is_none().then(|| first.clone())
}

fn contains_path(text: &str, path: &str) -> bool {
    text.match_indices(path).any(|(idx, _)| {
        let before = text[..idx].chars().next_back();
        let after = text[idx + path.len()..].chars().next();
        before.is_none_or(|ch| !is_path_char(ch)) && after.is_none_or(|ch| !is_path_char(ch))
    })
}

fn is_path_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '/' | '\\' | '_' | '-' | '.')
}

/// Repo-relative display path with `/` separators.
pub(crate) fn relative_label(path: &Path, root: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Expands `{file}`, `{line}`, `{column}`, and `{root}` in the command template.
pub(crate) fn expand_open_file_argv(
    template: &[String],
    file: &str,
    line: Option<u32>,
    column: Option<u32>,
    root: &Path,
) -> Vec<String> {
    let line = line.unwrap_or(1).max(1).to_string();
    let column = column.unwrap_or(1).max(1).to_string();
    let root = root.display().to_string();
    template
        .iter()
        .map(|arg| {
            arg.replace("{line}", &line)
                .replace("{column}", &column)
                .replace("{root}", &root)
                .replace("{file}", file)
        })
        .collect()
}

/// Returns true when `program` is an executable file on `PATH`.
pub(crate) fn program_on_path(program: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: &str, needle: &str) -> Option<FileRef> {
        let byte = line.find(needle).expect("needle in line");
        file_ref_at_char(line, line[..byte].chars().count())
    }

    fn file_ref(path: &str, line: Option<u32>, column: Option<u32>) -> FileRef {
        FileRef {
            path: path.to_string(),
            line,
            column,
        }
    }

    #[test]
    fn finds_claude_style_bare_name_with_line() {
        let line = "1. Opening prompt runs on more than the first turn (intent.py:303). It's";
        assert_eq!(
            at(line, "intent"),
            Some(file_ref("intent.py", Some(303), None))
        );
        assert_eq!(
            at(line, "303"),
            Some(file_ref("intent.py", Some(303), None))
        );
    }

    #[test]
    fn finds_opencode_style_relative_path_with_range() {
        let line = "PR #623 has one P2 finding at power_analyst/agent/workbook/intent.py:69-73.";
        assert_eq!(
            at(line, "workbook/"),
            Some(file_ref(
                "power_analyst/agent/workbook/intent.py",
                Some(69),
                None
            ))
        );
    }

    #[test]
    fn parses_columns_backticks_and_github_anchors() {
        assert_eq!(
            at("see `src/app/mod.rs:22:5`, then", "mod"),
            Some(file_ref("src/app/mod.rs", Some(22), Some(5)))
        );
        assert_eq!(
            at("src/lib.rs#L10-L20", "lib"),
            Some(file_ref("src/lib.rs", Some(10), None))
        );
        assert_eq!(
            at("[mod.rs:7]", "mod"),
            Some(file_ref("mod.rs", Some(7), None))
        );
    }

    #[test]
    fn accepts_bare_paths_only_with_a_separator() {
        assert_eq!(
            at("edited src/app/file_links.rs today", "file_links"),
            Some(file_ref("src/app/file_links.rs", None, None))
        );
        assert_eq!(at("edited README.md today", "README"), None);
    }

    #[test]
    fn rejects_non_paths() {
        assert_eq!(at("listening on 127.0.0.1:8080", "127"), None);
        assert_eq!(at("open https://example.com/a.rs:3", "example"), None);
        assert_eq!(at("ratio 3:2 wins", "3:2"), None);
        assert_eq!(at("a  b", " "), None);
        assert_eq!(at("(intent.py:303)", "("), None);
    }

    #[test]
    fn parses_file_uri_hyperlinks() {
        assert_eq!(
            parse_file_ref("file:///tmp/my%20dir/a.rs"),
            Some(file_ref("/tmp/my dir/a.rs", None, None))
        );
        assert_eq!(
            parse_file_ref("file://host/tmp/a.rs:12"),
            Some(file_ref("/tmp/a.rs", Some(12), None))
        );
    }

    #[test]
    fn suffix_pathspec_rejects_globs_and_parent_dirs() {
        assert_eq!(
            suffix_pathspec("./a/b.py").as_deref(),
            Some(":(glob)**/a/b.py")
        );
        assert_eq!(suffix_pathspec("*.py"), None);
        assert_eq!(suffix_pathspec("../b.py"), None);
        assert_eq!(suffix_pathspec("/abs/b.py"), None);
        assert_eq!(suffix_pathspec("C:x.py"), None);
    }

    fn temp_tree(name: &str, files: &[&str]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("herdr-file-links-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
            std::fs::write(&path, "x").expect("write file");
        }
        root
    }

    #[test]
    fn resolve_prefers_cwd_then_root_then_search() {
        let root = temp_tree("resolve", &["pkg/a.py", "pkg/sub/b.py", "c.py"]);
        let cwd = root.join("pkg");
        let no_search = |_: &Path, _: &str| -> Vec<PathBuf> { panic!("no search expected") };

        assert_eq!(
            resolve("a.py", &cwd, &root, None, no_search),
            Resolution::One(cwd.join("a.py"))
        );
        assert_eq!(
            resolve("c.py", &cwd, &root, None, no_search),
            Resolution::One(root.join("c.py"))
        );
        assert_eq!(
            resolve("b.py", &cwd, &root, None, |_, raw| {
                assert_eq!(raw, "b.py");
                vec![root.join("pkg/sub/b.py")]
            }),
            Resolution::One(root.join("pkg/sub/b.py"))
        );
        assert_eq!(
            resolve("/nope/missing.py", &cwd, &root, None, no_search),
            Resolution::None
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn suffix_candidates_lists_tracked_and_untracked_matches() {
        let root = temp_tree(
            "suffix",
            &[
                "agent/runtime/nodes/workbook.py",
                "contracts/workbook.py",
                "other.py",
                "ignored/workbook.py",
            ],
        );
        std::fs::write(root.join(".gitignore"), "ignored/\n").expect("write gitignore");
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .status()
            .expect("run git init");
        assert!(status.success());

        assert_eq!(
            suffix_candidates(&root, "workbook.py"),
            vec![
                root.join("agent/runtime/nodes/workbook.py"),
                root.join("contracts/workbook.py"),
            ]
        );
        assert_eq!(
            suffix_candidates(&root, "nodes/workbook.py"),
            vec![root.join("agent/runtime/nodes/workbook.py")]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scrollback_hint_needs_exactly_one_full_path() {
        let root = PathBuf::from("/repo");
        let candidates = vec![
            root.join("agent/runtime/nodes/workbook.py"),
            root.join("contracts/workbook.py"),
        ];
        assert_eq!(
            candidate_named_in_text(
                &candidates,
                &root,
                "Read agent/runtime/nodes/workbook.py\n... (workbook.py:185)"
            ),
            Some(root.join("agent/runtime/nodes/workbook.py"))
        );
        assert_eq!(
            candidate_named_in_text(
                &candidates,
                &root,
                "agent/runtime/nodes/workbook.py and contracts/workbook.py"
            ),
            None
        );
        // A longer path that merely ends with a candidate does not count.
        assert_eq!(
            candidate_named_in_text(&candidates, &root, "vendor/contracts/workbook.py"),
            None
        );
    }

    #[test]
    fn expands_open_file_template() {
        let template = ["code", "{root}", "-g", "{file}:{line}:{column}"].map(String::from);
        assert_eq!(
            expand_open_file_argv(&template, "/r/a.py", Some(22), None, Path::new("/r")),
            vec!["code", "/r", "-g", "/r/a.py:22:1"]
        );
        assert_eq!(
            expand_open_file_argv(&template, "/r/a.py", None, Some(0), Path::new("/r")),
            vec!["code", "/r", "-g", "/r/a.py:1:1"]
        );
    }
}
