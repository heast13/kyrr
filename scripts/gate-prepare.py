#!/usr/bin/env python3
"""Prepares one run of the release gate: everything the review crew reads,
made from one fixed commit and made on this machine.

Reviewers used to read a checkout that moved while they worked (fixes were
pushed during the review) and could not fetch the CI pictures (the cloud
proxy blocks the artifact host). So one run gets:

- a detached git worktree of the release commit (and of the plugin's), the
  only place the crew reads from;
- the output of `release-gate.py --release` for exactly that commit;
- the layout check's pictures, built here with the same example CI runs.

Everything lands in one folder with a PINNED.md that names the commit; the
crew's briefs say to read nothing else. After a fix, run this again with the
new commit and start only the areas the fix touches.

  python3 scripts/gate-prepare.py --plugin ../omarchy-kyrr
  python3 scripts/gate-prepare.py --ref 1af65bc --out /tmp/gate-0.8.1 --apt
  python3 scripts/gate-prepare.py --clean /tmp/gate-0.8.1

Standard library only. The layout part needs the Linux build libraries
(`--apt` installs them with apt-get) and downloads the assets once.
"""

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
APT = [
    "libpipewire-0.3-dev",
    "libclang-dev",
    "libasound2-dev",
    "libfontconfig1-dev",
    "libxkbcommon-dev",
    "libssl-dev",
    "pkg-config",
]


def git(directory, *args, check=True):
    """The output of a git command in a directory (stripped), or None on failure when not checked."""
    run = subprocess.run(["git", "-C", str(directory), *args], capture_output=True, text=True)
    if run.returncode:
        if check:
            sys.exit(f"git {' '.join(args)} in {directory}: {run.stderr.strip()}")
        return None
    return run.stdout.strip()


def worktree(repo, sha, target):
    """A detached worktree of `sha` at `target` (reused when it is already there)."""
    if (target / ".git").exists():
        if git(target, "rev-parse", "HEAD") == sha:
            return
        sys.exit(f"{target} is a worktree of another commit; pick another --out or --clean it")
    git(repo, "worktree", "add", "--detach", str(target), sha)


def run_logged(command, log, cwd, env=None):
    """Runs a command with its output in a log file; the exit code."""
    with open(log, "w", encoding="utf-8") as out:
        out.write("$ " + " ".join(map(str, command)) + "\n")
        out.flush()
        return subprocess.run(command, cwd=cwd, env=env, stdout=out, stderr=subprocess.STDOUT).returncode


def clean(out):
    """Removes the worktrees of a run and its folder."""
    for name, repo in (("src", ROOT), ("plugin", None)):
        target = out / name
        if not (target / ".git").exists():
            continue
        common = git(target, "rev-parse", "--git-common-dir", check=False)
        repo = Path(common).resolve().parent if common else repo
        git(repo, "worktree", "remove", "--force", str(target), check=False)
    shutil.rmtree(out, ignore_errors=True)
    print(f"removed {out}")


def main():
    parser = argparse.ArgumentParser(description="Prepares one pinned run of the release gate.")
    parser.add_argument("--ref", default="HEAD", help="the release commit (default HEAD of this checkout)")
    parser.add_argument("--previous", metavar="TAG", help="the previous release's tag (default: the newest tag before the commit)")
    parser.add_argument("--plugin", metavar="DIR", help="a checkout of heast13/omarchy-kyrr")
    parser.add_argument("--plugin-ref", default="HEAD", help="the plugin commit (default HEAD of its checkout)")
    parser.add_argument("--out", metavar="DIR", help="where the run goes (default: a new folder in the temp dir)")
    parser.add_argument("--online", action="store_true", help="release-gate.py also requests every external link")
    parser.add_argument("--no-layout", action="store_true", help="skip the layout pictures")
    parser.add_argument("--apt", action="store_true", help="install the Linux build libraries with apt-get first")
    parser.add_argument("--clean", metavar="DIR", help="remove a run made earlier and stop")
    args = parser.parse_args()
    if args.clean:
        clean(Path(args.clean).resolve())
        return

    sha = git(ROOT, "rev-parse", "--verify", f"{args.ref}^{{commit}}")
    out = Path(args.out).resolve() if args.out else Path(tempfile.mkdtemp(prefix=f"kyrr-gate-{sha[:12]}-"))
    out.mkdir(parents=True, exist_ok=True)
    src = out / "src"
    worktree(ROOT, sha, src)
    subject = git(src, "log", "-1", "--format=%s")
    version = tomllib.loads((src / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    previous = args.previous
    if not previous:
        previous = git(src, "describe", "--tags", "--abbrev=0", f"{sha}~1", check=False)
    if not previous:
        git(ROOT, "fetch", "--tags", "--quiet", "origin", check=False)  # a fresh clone may have no tags
        previous = git(src, "describe", "--tags", "--abbrev=0", f"{sha}~1", check=False)

    plugin_sha = None
    if args.plugin:
        plugin_repo = Path(args.plugin).resolve()
        plugin_sha = git(plugin_repo, "rev-parse", "--verify", f"{args.plugin_ref}^{{commit}}")
        worktree(plugin_repo, plugin_sha, out / "plugin")

    # The mechanical rules, for exactly this commit.
    gate_cmd = [sys.executable, "scripts/release-gate.py", "--release"]
    if args.online:
        gate_cmd.append("--online")
    if plugin_sha:
        gate_cmd += ["--plugin", str(out / "plugin")]
    gate_code = run_logged(gate_cmd, out / "release-gate.txt", src)
    print(f"release-gate.py: exit {gate_code}, output in {out / 'release-gate.txt'}")

    layout_note = "skipped (--no-layout)"
    if not args.no_layout:
        if args.apt and shutil.which("apt-get"):
            prefix = [] if os.geteuid() == 0 else ["sudo"]
            run_logged([*prefix, "apt-get", "install", "-y", "-qq", *APT], out / "apt.log", ROOT)
        # Assets and build cache are shared with this checkout: the worktree only has the sources.
        assets = ROOT / "assets"
        if not (assets / "sherpa-onnx" / "lib").exists():
            run_logged([sys.executable, "scripts/fetch-assets.py", "--linux"], out / "assets.log", ROOT)
        if not (src / "assets").exists() and assets.exists():
            (src / "assets").symlink_to(assets)
        env = dict(os.environ)
        env["SHERPA_ONNX_LIB_DIR"] = str(assets / "sherpa-onnx" / "lib")
        env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target"))
        code = run_logged(
            ["cargo", "run", "--release", "--locked", "--example", "layout", "--", str(out / "layout")],
            out / "layout.log",
            src,
            env,
        )
        problems = out / "layout" / "problems.txt"
        count = len(problems.read_text(encoding="utf-8").splitlines()) if problems.exists() else None
        if count is None:
            layout_note = f"FAILED to run (exit {code}), see layout.log"
        else:
            sheets = len(list((out / "layout").glob("sheet-*.png")))
            layout_note = f"{count} problem(s), {sheets} contact sheets (exit {code})"
        print(f"layout: {layout_note}")

    now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    pinned = f"""# Pinned run of the release gate

Made {now} for kyrr {version} (previous: {previous or 'unknown, pass --previous TAG'}).

- Commit: `{sha}` ({subject})
- Plugin commit: `{plugin_sha or 'not given'}`

**Read only from this folder.** The live checkout moves while fixes are
pushed; this run does not.

- Sources of kyrr at that commit: `{src}`
- Sources of the plugin at its commit: `{out / 'plugin' if plugin_sha else 'not given'}`
- `scripts/release-gate.py --release` for that commit: `{out / 'release-gate.txt'}` (exit {gate_code})
- Layout check, built on this machine, no CI artifact needed: `{out / 'layout'}`
  (`problems.txt`, contact sheets `sheet-<look>-<lang>-<scale>-<n>.png` with a
  `.txt` beside each that names the numbered pages): {layout_note}
- Diff of this release: {f"`git -C {src} diff {previous}..{sha}`" if previous else "unknown without the previous tag, run with --previous TAG"}

The cloud proxy lets neither the CI artifacts nor some external hosts
through: what cannot be fetched from here is checked from another machine,
and the report says so.

After a fix: run `scripts/gate-prepare.py` again with the new commit, start
only the areas the fix touches, and `--clean` this folder when the gate is done.
"""
    (out / "PINNED.md").write_text(pinned, encoding="utf-8")
    print(f"\npinned run: {out / 'PINNED.md'}")
    sys.exit(1 if gate_code else 0)


if __name__ == "__main__":
    main()
