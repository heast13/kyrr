# AUR

`kyrr-bin` installs the release tarball (`kyrr-<version>-linux-x86_64.tar.gz`)
on Arch Linux. `.github/workflows/aur.yml` builds it with `makepkg` on Arch,
checks it with `namcap` and installs it once, for every pull request that
touches this folder.

## Every release

Nothing to do by hand once the secret `AUR_SSH_PRIVATE_KEY` exists: when a
release is published, the workflow runs `update.sh <version>` (new `pkgver`
and checksum from the release's `.sha256` file, `.SRCINFO` from
`makepkg --printsrcinfo`), builds and checks the package and pushes
`PKGBUILD` and `.SRCINFO` to `ssh://aur@aur.archlinux.org/kyrr-bin.git`.
Without the secret it says so and pushes nothing. For an older release, run
the workflow by hand with its tag.

## The first upload

The AUR repository is created by the first push, from the maintainer's
account:

1. An account on <https://aur.archlinux.org/register> with a public SSH key
   (My Account, "SSH Public Key"). A key only for the AUR:
   `ssh-keygen -t ed25519 -f ~/.ssh/aur -C aur`.
2. Push this folder's two files:

   ```sh
   GIT_SSH_COMMAND="ssh -i ~/.ssh/aur" git clone ssh://aur@aur.archlinux.org/kyrr-bin.git /tmp/kyrr-bin
   cp packaging/aur/kyrr-bin/PKGBUILD packaging/aur/kyrr-bin/.SRCINFO /tmp/kyrr-bin/
   cd /tmp/kyrr-bin && git add PKGBUILD .SRCINFO && git commit -m "kyrr-bin 0.X.Y"
   GIT_SSH_COMMAND="ssh -i ~/.ssh/aur" git push origin HEAD:master
   ```

   The clone of a new name is empty, with a warning; that is expected.
3. The private key (`~/.ssh/aur`, the whole file) as the repository secret
   `AUR_SSH_PRIVATE_KEY` (Settings, Secrets and variables, Actions). From
   then on releases go to the AUR by themselves.
