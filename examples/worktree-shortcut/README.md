# Create and open a worktree

A local Zsh shortcut using Herdr's existing popup and CLI support. Requires
Git, Zsh, `fzf`, and Herdr with popup commands and `herdr worktree open`.
No Herdr rebuild is needed.

## Install

From this directory, copy the two scripts to their expected locations. If those
files already exist, compare them first and preserve any local changes.

```sh
mkdir -p ~/.config/zsh ~/.config/herdr
cp wt.zsh ~/.config/zsh/wt.zsh
cp worktree-new.zsh ~/.config/herdr/worktree-new.zsh
```

Source the shared function from `~/.zshrc`, replacing any existing `wt` function:

```sh
source "$HOME/.config/zsh/wt.zsh"
```

Append the table in `config.toml` to `~/.config/herdr/config.toml`. Choose another
key if `prefix+t` is already assigned, then apply the configuration:

```sh
herdr server reload-config
```

## Use

From the repository's main checkout in Herdr, press your prefix followed by
lowercase `t` (by default, Ctrl+B then T). Enter the new branch name; a blank
name cancels.

- If the branch exists on `origin`, fetch it and create a tracking worktree.
- Otherwise, select a local base branch in `fzf`, with `develop` listed first
  when present. With only one base branch, `fzf` selects it automatically.
- The checkout is created under the current repository's `.worktrees/`
  directory, with slashes in branch names replaced by underscores.
- Herdr opens and focuses the resulting worktree automatically.

Existing local branch names and existing destination paths are rejected without
overwriting them. Git uses the repository's configured `origin` and normal
authentication. The scripts do not push code or run the branch name as a shell
command. If opening in Herdr fails after creation, the checkout is kept and the
error stays visible until Enter is pressed.

The terminal command `wt <new-branch>` still creates a worktree using the same
flow, without opening it in Herdr.

## Check

```sh
zsh -n wt.zsh worktree-new.zsh
python3 test_worktree_shortcut.py
```

The test uses temporary local repositories, a local bare origin, and fake
Herdr/`fzf` executables. It does not contact GitHub or change running sessions.
