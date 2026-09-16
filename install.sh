#!/usr/bin/env bash
# afanctl — one-command installer (Arch Linux / Omarchy, x86_64).
#
#   curl -fsSL https://github.com/yadav-prakhar/afanctl/releases/latest/download/install.sh | sudo bash
#
# Downloads the package attached to a GitHub release, checks its SHA-256
# against that release's SHA256SUMS, and installs it with pacman. Nothing is
# enabled, started or written to /sys: the unit installs disabled and the
# daemon starts in observe mode, so the fan stays on the firmware curve until
# you ask for `curve` or `hold`.
#
#   --version <vX.Y.Z>   install a specific release instead of the latest
#   --dry-run            resolve, download and verify — install nothing
#   -h, --help           this text
#
# Exit codes: 0 success · 1 refused/failed · 2 usage error.
set -euo pipefail

repo="yadav-prakhar/afanctl"
# The one asset we install. `^afanctl-[0-9]` excludes the debug split that
# makepkg also produces. Brackets rather than escapes: awk warns on `\.` in a
# -v value.
asset_re='^afanctl-[0-9][^[:space:]]*-x86_64[.]pkg[.]tar[.]zst$'

version=""
dry_run=0

die() {
    printf 'afanctl: %s\n' "$1" >&2
    exit 1
}

usage() {
    # Lines 2-16 of this file are the usage text.
    if [ -f "${BASH_SOURCE[0]:-}" ]; then
        sed -n '2,16p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    else
        cat <<'EOF'
afanctl installer

  curl -fsSL https://github.com/yadav-prakhar/afanctl/releases/latest/download/install.sh | sudo bash

  --version <vX.Y.Z>   install a specific release instead of the latest
  --dry-run            resolve, download and verify — install nothing
  -h, --help           this text
EOF
    fi
}

while [ $# -gt 0 ]; do
    case "$1" in
        -V | --version)
            [ $# -ge 2 ] || die "--version needs a value (e.g. --version 0.1.0)"
            version="$2"
            shift 2
            ;;
        -n | --dry-run)
            dry_run=1
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            printf 'afanctl: unknown argument: %s\n\n' "$1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

[ "$(uname -m)" = "x86_64" ] ||
    die "this package targets x86_64 (applesmc/coretemp are Intel-only); got $(uname -m)"

command -v pacman >/dev/null 2>&1 ||
    die "pacman not found — this installer targets Arch Linux/Omarchy. Build from source instead:
    git clone https://github.com/$repo && cd afanctl/packaging && makepkg -si"

# Elevate when we can do it transparently; otherwise say exactly what to run.
if [ "$(id -u)" -ne 0 ] && [ "$dry_run" -eq 0 ]; then
    if [ -f "${BASH_SOURCE[0]:-}" ] && command -v sudo >/dev/null 2>&1; then
        args=()
        [ -n "$version" ] && args+=(--version "$version")
        echo "afanctl: installing needs root — re-running with sudo"
        exec sudo -- bash "${BASH_SOURCE[0]}" "${args[@]}"
    fi
    die "installing needs root — re-run this with sudo (the command on the download page already does)"
fi

if [ -n "$version" ]; then
    case "$version" in
        v*) ;;
        *) version="v$version" ;;
    esac
    base="https://github.com/$repo/releases/download/$version"
else
    base="https://github.com/$repo/releases/latest/download"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL --retry 3 --retry-delay 2 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO "$2" "$1"; }
else
    die "neither curl nor wget is installed"
fi

echo "afanctl: release   $base"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" ||
    die "could not fetch SHA256SUMS — does that release exist? (see https://github.com/$repo/releases)"

asset="$(awk -v re="$asset_re" '$2 ~ re { print $2; exit }' "$tmp/SHA256SUMS")"
[ -n "$asset" ] ||
    die "no x86_64 package listed in the release's SHA256SUMS — refusing to guess"

sum="$(awk -v name="$asset" '$2 == name { print $1 }' "$tmp/SHA256SUMS")"
[ -n "$sum" ] || die "no checksum for $asset in SHA256SUMS"

echo "afanctl: package   $asset"
echo "afanctl: sha256    $sum"
fetch "$base/$asset" "$tmp/$asset" || die "could not download $asset"

# Mandatory: a mismatch aborts here (sha256sum -c fails, set -e exits).
printf '%s  %s\n' "$sum" "$tmp/$asset" | sha256sum -c -

if [ "$dry_run" -eq 1 ]; then
    echo "afanctl: dry run — verified, installing nothing. Drop --dry-run to install."
    exit 0
fi

pacman -U "$tmp/$asset"

installed="$(pacman -Q afanctl 2>/dev/null || echo 'afanctl (not registered?)')"
cat <<EOF

afanctl: installed — $installed
  binary   /usr/bin/afanctl
  unit     /usr/lib/systemd/system/afanctl.service   (installs DISABLED)
  config   /etc/afanctl/afanctl.toml                 (pacman backup= — your edits survive upgrades)
  default  /usr/share/afanctl/afanctl.toml.default
  polkit   /usr/share/polkit-1/rules.d/49-afanctl.rules   (wheel: status/observe/curve/hold)

  The fan is still on the firmware curve. Nothing changes until you opt in:

    sudo systemctl enable --now afanctl     # starts in observe mode: reads, writes nothing
    afanctl status                          # temps, mode, fan, config provenance
    sudo afanctl doctor                     # 10 checks; run as root, exit 1 on any FAIL

  Opt into control   sudo afanctl curve | sudo afanctl hold 3000 | sudo afanctl observe
  Re-run this script to upgrade · sudo pacman -R afanctl to remove
  Docs: https://github.com/$repo/wiki
EOF
