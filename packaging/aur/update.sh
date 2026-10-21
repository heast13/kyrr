#!/bin/bash
# Points the kyrr-bin package at a published release:
#   packaging/aur/update.sh 0.4.1
# Takes the checksum from the release's own .sha256 file and rewrites PKGBUILD
# and .SRCINFO. Then push packaging/aur/kyrr-bin to the AUR (git, ssh://aur@aur.archlinux.org/kyrr-bin.git).
set -euo pipefail
ver=${1:?usage: update.sh <version>}
dir=$(cd "$(dirname "$0")/kyrr-bin" && pwd)
file=kyrr-$ver-linux-x86_64.tar.gz
base=https://github.com/heast13/kyrr/releases/download/v$ver
read -r sum name < <(curl -fsSL "$base/$file.sha256")
[[ $sum =~ ^[0-9a-f]{64}$ && ${name#\*} == "$file" ]] || { echo "unexpected $file.sha256 on the release" >&2; exit 1; }
sed -i -e "s/^pkgver=.*/pkgver=$ver/" -e 's/^pkgrel=.*/pkgrel=1/' \
  -e "s/^sha256sums=.*/sha256sums=('$sum')/" "$dir/PKGBUILD"
if command -v makepkg >/dev/null; then
  (cd "$dir" && makepkg --printsrcinfo > .SRCINFO)
else
  sed -i -e "s/^\tpkgver = .*/\tpkgver = $ver/" -e 's/^\tpkgrel = .*/\tpkgrel = 1/' \
    -e "s|^\tsource = .*|\tsource = $base/$file|" -e "s/^\tsha256sums = .*/\tsha256sums = $sum/" "$dir/.SRCINFO"
fi
echo "kyrr-bin $ver, sha256 $sum"
