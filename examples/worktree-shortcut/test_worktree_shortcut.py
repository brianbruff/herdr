"""Run with python3; uses disposable local Git repos and a fake Herdr client."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


script = Path(__file__).resolve().with_name("worktree-new.zsh")
helper = script.with_name("wt.zsh")


with tempfile.TemporaryDirectory(prefix="herdr-wt-test-") as tmp:
    root = Path(tmp).resolve()
    repo = root / "repo with spaces"
    repo.mkdir()

    def git(*args, cwd=repo):
        return subprocess.check_output(
            ["git", *args], cwd=cwd, stderr=subprocess.PIPE, text=True
        ).strip()

    git("init", "-b", "develop")
    git("config", "user.name", "Worktree test")
    git("config", "user.email", "worktree-test@example.invalid")
    (repo / "base.txt").write_text("develop\n")
    git("add", ".")
    git("commit", "-m", "base")
    base = git("rev-parse", "HEAD")
    git("switch", "-c", "remote/feature")
    (repo / "remote.txt").write_text("remote branch\n")
    git("add", ".")
    git("commit", "-m", "remote feature")
    remote_head = git("rev-parse", "HEAD")
    git("switch", "-c", "other-base", "develop")
    (repo / "other.txt").write_text("other base\n")
    git("add", ".")
    git("commit", "-m", "other base")
    origin = root / "origin.git"
    git("clone", "--bare", str(repo), str(origin))
    git("branch", "-D", "remote/feature")
    git("remote", "add", "origin", str(origin))

    shared = root / ".config/zsh/wt.zsh"
    shared.parent.mkdir(parents=True)
    shutil.copyfile(helper, shared)
    bin_dir = root / "bin"
    bin_dir.mkdir()
    log = root / "calls.jsonl"
    herdr = bin_dir / "herdr"
    herdr.write_text(
        "#!/usr/bin/env python3\nimport json, os, sys\n"
        "with open(os.environ['TEST_LOG'], 'a') as f: "
        "f.write(json.dumps(sys.argv[1:]) + '\\n')\n"
        "sys.exit(1 if os.environ.get('FAIL_ACTION') == sys.argv[2] else 0)\n"
    )
    fzf = bin_dir / "fzf"
    fzf.write_text(
        "#!/usr/bin/env python3\nimport os, sys\n"
        "choices = sys.stdin.read().splitlines()\n"
        "if os.environ.get('CANCEL_BASE'): sys.exit(130)\n"
        "print(choices[0])\n"
    )
    herdr.chmod(0o755)
    fzf.chmod(0o755)
    env = dict(os.environ, HOME=str(root), PATH=f"{bin_dir}:{os.environ['PATH']}",
               HERDR_BIN_PATH=str(herdr), TEST_LOG=str(log))

    def run(branch, *, success=True, **extra):
        log.write_text("")
        result = subprocess.run(
            ["/bin/zsh", str(script)], cwd=repo, env=dict(env, **extra),
            input=branch + "\n\n", text=True, capture_output=True,
        )
        assert (result.returncode == 0) == success, result.stdout + result.stderr
        calls = [json.loads(line) for line in log.read_text().splitlines()]
        assert calls[0] == ["worktree", "list", "--cwd", str(repo)]
        return calls

    calls = run("feature/new")
    assert calls[-1] == ["worktree", "open", "--cwd", str(repo),
                         "--branch", "feature/new", "--focus"]
    assert git("rev-parse", "feature/new") == base
    assert (repo / ".worktrees/feature_new/base.txt").is_file()
    assert len(run("feature/$(touch${IFS}PWNED)")) == 2
    assert not (repo / "PWNED").exists()
    assert not (root / "PWNED").exists()
    assert len(run("remote/feature")) == 2
    assert git("rev-parse", "remote/feature") == remote_head
    assert git("rev-parse", "--abbrev-ref", "remote/feature@{upstream}") == "origin/remote/feature"
    assert len(run("")) == 1
    assert len(run("bad name", success=False)) == 1
    assert len(run("develop", success=False)) == 1
    assert len(run("cancelled", success=False, CANCEL_BASE="1")) == 1
    assert len(run("preflight-failed", success=False, FAIL_ACTION="list")) == 1
    assert len(run("open-failed", success=False, FAIL_ACTION="open")) == 2
    assert (repo / ".worktrees/open-failed").is_dir()
    git("remote", "set-url", "origin", str(root / "missing-origin.git"))
    assert len(run("remote-failed", success=False)) == 1
    for branch in ["cancelled", "preflight-failed", "remote-failed"]:
        assert not (repo / ".worktrees" / branch).exists()

print("Passed: new/develop base, remote tracking, cancellation, validation, and failure handling.")
