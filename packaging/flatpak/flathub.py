"""Writes the files for Flathub for a released version of kyrr.

Usage: python3 packaging/flatpak/flathub.py <version | commit> <out dir>

<out dir> gets io.github.heast13.kyrr.yml, cargo-sources.json and
flathub.json: the manifest here with the working-tree source swapped for the
release's git tag and commit (Flathub builds from that), and the other two
files as they are in that tag. These three files are all the Flathub
repository holds. Needs git and the tag `v<version>` (fetched from GitHub
when it is not here). With a full commit hash instead of a version, the
source is that commit of kyrr's GitHub repository: the CI builds a pull
request the way Flathub would build it.
"""
import os
import re
import subprocess
import sys

URL = "https://github.com/heast13/kyrr.git"
HERE = "packaging/flatpak"


def git(*args, text=True):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=text).stdout


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    version, out = sys.argv[1], sys.argv[2]
    os.chdir(git("rev-parse", "--show-toplevel").strip())
    if re.fullmatch(r"[0-9a-f]{40}", version):
        tag, commit = None, version
    else:
        tag = f"v{version}"
        try:
            commit = git("rev-parse", f"{tag}^{{commit}}").strip()
        except subprocess.CalledProcessError:
            git("fetch", "--quiet", URL, f"refs/tags/{tag}:refs/tags/{tag}")
            commit = git("rev-parse", f"{tag}^{{commit}}").strip()

    manifest = git("show", f"{commit}:{HERE}/io.github.heast13.kyrr.yml")
    # The dir source with its comment, up to the cargo sources.
    local = re.compile(r"      # Local builds:.*?\n(?=      - cargo-sources\.json\n)", re.S)
    if len(local.findall(manifest)) != 1:
        sys.exit(f"{tag or commit}: the manifest's local source block was not found")
    source = f"      - type: git\n        url: {URL}\n"
    if tag:
        source += f"        tag: {tag}\n"
    source += f"        commit: {commit}\n"
    manifest = local.sub(lambda _: source, manifest)

    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "io.github.heast13.kyrr.yml"), "w", encoding="utf-8") as f:
        f.write(manifest)
    for name in ("cargo-sources.json", "flathub.json"):
        with open(os.path.join(out, name), "wb") as f:
            f.write(git("show", f"{commit}:{HERE}/{name}", text=False))
    print(f"{tag or 'commit'} ({commit}) -> {out}")


if __name__ == "__main__":
    main()
