#!/bin/zsh

source "$HOME/.config/zsh/wt.zsh" || exit 1

create_and_open_worktree() {
  local repo_root branch
  local herdr_bin="${HERDR_BIN_PATH:-herdr}"
  repo_root=$(git rev-parse --show-toplevel) || return 1

  # Check the source before creating anything; Herdr requires the parent checkout.
  "$herdr_bin" worktree list --cwd "$repo_root" >/dev/null || return 1
  read -r "branch?Branch (blank cancels)> " || return 0
  [[ -n "$branch" ]] || return 0

  wt "$branch" || return $?
  "$herdr_bin" worktree open --cwd "$repo_root" --branch "$branch" --focus
}

create_and_open_worktree
worktree_result=$?
if (( worktree_result != 0 )); then
  read -r '?Press Enter to close...'
fi
exit "$worktree_result"
