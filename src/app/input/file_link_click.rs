//! Ctrl+click on a file reference opens it with `[editor] open_file`.

use std::path::{Path, PathBuf};

use crate::app::file_links::{self, FileRef, Resolution};
use crate::app::App;

/// Stand-in for `{file}` while building the picker script; the script swaps
/// in the path the user picks.
const PICKER_FILE_MARKER: &str = "\u{0}herdr-picked-file\u{0}";

impl App {
    /// Returns true when the click landed on a resolvable file reference and
    /// Herdr took it; false lets the click reach the pane as usual.
    pub(super) fn handle_file_link_click(
        &mut self,
        pane_id: crate::layout::PaneId,
        viewport_row: u16,
        col: u16,
    ) -> bool {
        if self.state.open_file_command.is_empty() {
            return false;
        }
        let Some(file_ref) =
            self.state
                .file_ref_at_pane_cell(&self.terminal_runtimes, pane_id, viewport_row, col)
        else {
            return false;
        };
        let Some(cwd) = self.pane_cwd(pane_id) else {
            return false;
        };
        let root = crate::workspace::git_space_metadata(&cwd)
            .map(|metadata| metadata.repo_root)
            .unwrap_or_else(|| cwd.clone());
        let home = std::env::var_os("HOME").map(PathBuf::from);

        let path = match file_links::resolve(
            &file_ref.path,
            &cwd,
            &root,
            home.as_deref(),
            file_links::suffix_candidates,
        ) {
            Resolution::None => return false,
            Resolution::One(path) => path,
            Resolution::Many(candidates) => {
                let text = self.pane_recent_text(pane_id, file_links::SCROLLBACK_HINT_LINES);
                match file_links::candidate_named_in_text(&candidates, &root, &text) {
                    Some(path) => path,
                    None => {
                        self.pick_file_link(candidates, &file_ref, &root);
                        return true;
                    }
                }
            }
        };
        self.open_file_link(&path, &file_ref, &root);
        true
    }

    fn pane_cwd(&self, pane_id: crate::layout::PaneId) -> Option<PathBuf> {
        let ws = self.state.workspaces.get(self.state.active?)?;
        ws.tabs.iter().find_map(|tab| {
            tab.cwd_for_pane(pane_id, &self.state.terminals, &self.terminal_runtimes)
        })
    }

    fn pane_recent_text(&self, pane_id: crate::layout::PaneId, lines: usize) -> String {
        self.state
            .active
            .and_then(|ws_idx| {
                self.state
                    .runtime_for_pane_in_workspace(&self.terminal_runtimes, ws_idx, pane_id)
            })
            .map(|rt| rt.recent_text(lines))
            .unwrap_or_default()
    }

    fn open_file_link(&mut self, path: &Path, file_ref: &FileRef, root: &Path) {
        let argv = file_links::expand_open_file_argv(
            &self.state.open_file_command,
            &path.display().to_string(),
            file_ref.line,
            file_ref.column,
            root,
        );
        if let Err(err) = spawn_detached(&argv, root) {
            tracing::warn!(err = %err, argv = ?argv, "failed to run editor open_file command");
            self.show_file_link_toast(
                crate::app::state::ToastKind::NeedsAttention,
                "open file failed",
                format!("{}: {err}", argv.first().map_or("", String::as_str)),
            );
        }
    }

    fn pick_file_link(&mut self, candidates: Vec<PathBuf>, file_ref: &FileRef, root: &Path) {
        let labels: Vec<String> = candidates
            .iter()
            .filter_map(|candidate| file_links::relative_label(candidate, root))
            .collect();
        let open_argv = file_links::expand_open_file_argv(
            &self.state.open_file_command,
            PICKER_FILE_MARKER,
            file_ref.line,
            file_ref.column,
            root,
        );
        let picker = file_links::program_on_path("fzf")
            .then(|| {
                crate::platform::file_picker_argv(&labels, root, &open_argv, PICKER_FILE_MARKER)
                    .ok()
            })
            .flatten();

        let Some(picker) = picker else {
            // No picker available: open the first match and say so.
            let count = candidates.len();
            if let Some(first) = candidates.first() {
                self.open_file_link(first, file_ref, root);
            }
            self.show_file_link_toast(
                crate::app::state::ToastKind::Finished,
                "opened first match",
                format!("{count} files match {}; install fzf to pick", file_ref.path),
            );
            return;
        };

        let (env, _) = self.custom_command_env();
        match self.spawn_overlay_argv_command(&picker, Some(root.to_path_buf()), env, Vec::new()) {
            Ok((_, new_pane)) => {
                let terminal_id = new_pane.terminal.id.clone();
                self.terminal_runtimes
                    .insert(terminal_id.clone(), new_pane.runtime);
                self.state
                    .remove_alias_shadowed_by_new_pane(new_pane.pane_id);
                self.state.terminals.insert(terminal_id, new_pane.terminal);
            }
            Err(err) => {
                tracing::warn!(err = %err, "failed to open file link picker");
                self.show_file_link_toast(
                    crate::app::state::ToastKind::NeedsAttention,
                    "file picker failed",
                    err.to_string(),
                );
            }
        }
    }

    fn show_file_link_toast(
        &mut self,
        kind: crate::app::state::ToastKind,
        title: &str,
        context: String,
    ) {
        let previous_toast = self.state.toast.clone();
        self.state.toast = Some(crate::app::state::ToastNotification {
            kind,
            title: title.to_string(),
            context,
            position: None,
            target: None,
        });
        self.sync_toast_deadline(previous_toast);
    }
}

/// Starts the editor without tying it to a pane; a thread reaps the child so
/// it does not linger as a zombie.
fn spawn_detached(argv: &[String], cwd: &Path) -> std::io::Result<()> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| std::io::Error::other("editor open_file command is empty"))?;
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    crate::platform::configure_background_command(&mut command);
    let mut child = command.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;

    use super::super::{app_for_mouse_test, unique_temp_path, wait_for_file};
    use crate::app::state::Mode;
    use crate::app::App;
    use crate::workspace::Workspace;

    fn temp_repo(name: &str, files: &[&str]) -> std::path::PathBuf {
        let root = unique_temp_path(name);
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
            std::fs::write(&path, "x").expect("write file");
        }
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .status()
            .expect("run git init");
        assert!(status.success());
        root.canonicalize().expect("canonical root")
    }

    /// Builds an app whose only pane shows `text` with `cwd` as its cwd, and
    /// an `open_file` command that records its argument in `out`.
    fn app_with_pane_text(text: &str, cwd: &std::path::Path, out: &std::path::Path) -> (App, Rect) {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        let pane_infos = ws.tabs[0].layout.panes(Rect::new(26, 2, 80, 18));
        let inner = pane_infos[0].inner_rect;
        ws.insert_test_runtime(
            pane_id,
            crate::terminal::TerminalRuntime::test_with_screen_bytes(
                inner.width,
                inner.height,
                text.as_bytes(),
            ),
        );
        let terminal_id = ws.tabs[0].panes[&pane_id].attached_terminal_id.clone();
        app.state.terminals.insert(
            terminal_id.clone(),
            crate::terminal::TerminalState::new(terminal_id, cwd.to_path_buf()),
        );
        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.view.pane_infos = pane_infos;
        app.state.open_file_command = [
            "/bin/sh",
            "-c",
            "printf '%s|%s' \"$1\" \"$PWD\" > \"$2\"",
            "sh",
            "{file}:{line}:{column}",
        ]
        .into_iter()
        .map(String::from)
        .chain([out.display().to_string()])
        .collect();
        (app, inner)
    }

    fn ctrl_click(app: &mut App, inner: Rect, text: &str, needle: &str) {
        let col = text.find(needle).expect("needle in pane text") as u16;
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: inner.x + col,
            row: inner.y,
            modifiers: KeyModifiers::CONTROL,
        });
    }

    #[tokio::test]
    async fn ctrl_click_opens_repo_relative_reference_at_line_and_column() {
        let root = temp_repo("file-link-relative", &["pkg/agent/intent.py"]);
        let out = root.join("out.txt");
        let text = "finding at pkg/agent/intent.py:69:4-73 is real";
        let (mut app, inner) = app_with_pane_text(text, &root.join("pkg"), &out);

        ctrl_click(&mut app, inner, text, "agent/");

        assert_eq!(
            wait_for_file(&out),
            format!(
                "{}:69:4|{}",
                root.join("pkg/agent/intent.py").display(),
                root.display()
            )
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn ctrl_click_finds_bare_name_and_uses_scrollback_to_disambiguate() {
        let root = temp_repo(
            "file-link-bare",
            &["runtime/nodes/workbook.py", "contracts/workbook.py"],
        );
        let out = root.join("out.txt");
        let text = "Read runtime/nodes/workbook.py\r\nloop (workbook.py:185) again";
        let (mut app, inner) = app_with_pane_text(text, &root, &out);

        let col = "loop (".len() as u16;
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: inner.x + col,
            row: inner.y + 1,
            modifiers: KeyModifiers::CONTROL,
        });

        assert_eq!(
            wait_for_file(&out),
            format!(
                "{}:185:1|{}",
                root.join("runtime/nodes/workbook.py").display(),
                root.display()
            )
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn ctrl_click_ignores_missing_files_and_unset_command() {
        let root = temp_repo("file-link-missing", &["present.py"]);
        let out = root.join("out.txt");
        let text = "see missing.py:3 and present.py:4";

        let (mut app, inner) = app_with_pane_text(text, &root, &out);
        assert!(!app.handle_file_link_click(app.state.workspaces[0].tabs[0].root_pane, 0, 4));

        app.state.open_file_command.clear();
        let col = text.find("present").expect("present") as u16;
        assert!(!app.handle_file_link_click(app.state.workspaces[0].tabs[0].root_pane, 0, col));
        ctrl_click(&mut app, inner, text, "present");
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!out.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
