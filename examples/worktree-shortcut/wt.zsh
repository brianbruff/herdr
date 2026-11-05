# Create a project-local Git worktree, reusing an existing origin branch when present.
wt() {
  if (( $# != 1 )); then
    echo "Usage: wt <new-branch>" >&2
    return 2
  fi

  if ! command -v fzf >/dev/null 2>&1; then
    echo "wt: fzf is required" >&2
    return 1
  fi

  local branch="$1"
  if ! git check-ref-format --branch "$branch" >/dev/null 2>&1; then
    echo "wt: invalid branch name: $branch" >&2
    return 1
  fi

  local repo_root
  repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || {
    echo "wt: run this from inside a Git repository" >&2
    return 1
  }

  if git -C "$repo_root" show-ref --verify --quiet "refs/heads/$branch"; then
    echo "wt: local branch already exists: $branch" >&2
    return 1
  fi

  local safe_branch="${branch//\//_}"
  local worktrees_dir="$repo_root/.worktrees"
  local worktree_path="$worktrees_dir/$safe_branch"

  if [[ -e "$worktree_path" ]]; then
    echo "wt: destination already exists: $worktree_path" >&2
    return 1
  fi

  if git -C "$repo_root" remote get-url origin >/dev/null 2>&1; then
    git -C "$repo_root" ls-remote --exit-code --heads origin "refs/heads/$branch" >/dev/null 2>&1
    local remote_status=$?
    if (( remote_status == 0 )); then
      git -C "$repo_root" fetch origin \
        "refs/heads/${branch}:refs/remotes/origin/${branch}" || return 1
      mkdir -p "$worktrees_dir" || return 1
      git -C "$repo_root" worktree add --track -b "$branch" \
        "$worktree_path" "origin/$branch" || return 1
      echo "Created worktree from origin/$branch: $worktree_path"
      return 0
    elif (( remote_status != 2 )); then
      echo "wt: could not check origin for branch: $branch" >&2
      return 1
    fi
  fi

  local -a branches ordered_branches
  branches=("${(@f)$(git -C "$repo_root" for-each-ref \
    --format='%(refname:short)' refs/heads)}")
  if (( ${branches[(Ie)develop]} )); then
    ordered_branches=(develop "${(@)branches:#develop}")
  else
    ordered_branches=("${branches[@]}")
  fi

  local base
  base=$(
    printf "%s\n" "${ordered_branches[@]}" |
      fzf \
        --prompt="Base branch> " \
        --height=40% \
        --reverse \
        --border \
        --header="Default: ${ordered_branches[1]}. Use arrows then Enter; Esc cancels." \
        --select-1 \
        --exit-0
  )
  [[ -n "$base" ]] || return 1

  mkdir -p "$worktrees_dir" || return 1
  git -C "$repo_root" worktree add -b "$branch" "$worktree_path" "$base" || return 1
  echo "Created worktree: $worktree_path"
}
