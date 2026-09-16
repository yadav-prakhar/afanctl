#!/usr/bin/env bash
# Point the AUR package at a released source tarball.
#
#   packaging/aur/update-aur.sh 0.1.1 [--tarball FILE] [--aur-dir DIR] [--push]
#
# What it does, in order:
#   1. resolves the release asset for <version> (downloads it unless --tarball
#      names a local file),
#   2. rewrites `pkgver` and `sha256sums` in this directory's PKGBUILD,
#   3. mirrors PKGBUILD + LICENSE + REUSE.toml into --aur-dir and regenerates
#      .SRCINFO there,
#   4. with --aur-dir, commits; with --push, pushes to the AUR.
#
# Steps 1-2 run here; step 3 needs an AUR clone (`git clone
# ssh://aur@aur.archlinux.org/afanctl.git`) because the AUR only accepts pushes
# over SSH with a key registered in your AUR account. See README.md.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
upstream="$(cd "$here/../.." && pwd)"

version=""
tarball=""
aur_dir=""
push=0

while [ $# -gt 0 ]; do
    case "$1" in
        --tarball) tarball="${2:?--tarball needs a path}"; shift 2 ;;
        --aur-dir) aur_dir="${2:?--aur-dir needs a path}"; shift 2 ;;
        --push) push=1; shift ;;
        -h | --help)
            sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        -*)
            echo "unknown option: $1" >&2
            exit 2
            ;;
        *)
            [ -n "$version" ] && { echo "version given twice" >&2; exit 2; }
            version="$1"
            shift
            ;;
    esac
done

[ -n "$version" ] || { echo "usage: $0 <version> [--tarball FILE] [--aur-dir DIR] [--push]" >&2; exit 2; }
case "$version" in
    v*) version="${version#v}" ;;
esac

repo_url="$(sed -n 's/^url="\?\([^"]*\)"\?$/\1/p' "$here/PKGBUILD" | sed -n '1p')"
[ -n "$repo_url" ] || { echo "could not read url= from $here/PKGBUILD" >&2; exit 1; }
asset_name="afanctl-$version.tar.gz"

tmp=""
if [ -z "$tarball" ]; then
    tmp="$(mktemp -d)"
    tarball="$tmp/$asset_name"
    url="$repo_url/releases/download/v$version/$asset_name"
    echo "downloading $url"
    curl -fSL --retry 3 -o "$tarball" "$url"
fi
[ -f "$tarball" ] || { echo "no such tarball: $tarball" >&2; exit 1; }

sum="$(sha256sum "$tarball" | awk '{print $1}')"
echo "sha256($tarball) = $sum"

sed -i -E \
    -e "s/^pkgver=.*/pkgver=$version/" \
    -e "s/^sha256sums=\(.*\)/sha256sums=('$sum')/" \
    "$here/PKGBUILD"

grep -E '^(pkgver|sha256sums)=' "$here/PKGBUILD"

if [ -n "$aur_dir" ]; then
    [ -d "$aur_dir/.git" ] || { echo "$aur_dir is not a git clone of the AUR repo" >&2; exit 1; }
    cp "$here/PKGBUILD" "$here/LICENSE" "$here/REUSE.toml" "$aur_dir/"
    (cd "$aur_dir" && makepkg --printsrcinfo > .SRCINFO)
    grep -E '^\s*(pkgver|sha256sums)' "$aur_dir/.SRCINFO"
    (cd "$aur_dir" && git add -A && git commit -m "upgpkg: afanctl $version-1")
    if [ "$push" -eq 1 ]; then
        (cd "$aur_dir" && git push origin master)
        echo "pushed afanctl $version-1 to the AUR"
    else
        echo "committed in $aur_dir — review it, then: git -C $aur_dir push origin master"
    fi
fi

[ -n "$tmp" ] && rm -rf "$tmp"
exit 0