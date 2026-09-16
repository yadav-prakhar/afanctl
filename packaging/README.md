# Publishing afanctl

How a release becomes installable, which channel is live, and what is staged for
the ones that are not.

## Channels

| Channel | Artifact | Status |
| ------- | -------- | ------ |
| **GitHub release** | source tarball, `install.sh`, `afanctl-<ver>-1-x86_64.pkg.tar.zst`, `SHA256SUMS` | **live — this is the distribution channel** |
| AUR (`afanctl`) | `packaging/aur/` (`PKGBUILD`, `.SRCINFO`, 0BSD `LICENSE`, `REUSE.toml`) | prepared, **not published**: the AUR has closed registration to new maintainers |
| Omarchy (`pkgs.omarchy.org`) | a recipe they own in `omacom/omarchy-pkgs`, watching our releases | proposed as [omacom/omarchy-pkgs#476](https://github.com/omacom/omarchy-pkgs/pull/476) — their merge decides it |
| crates.io | `Cargo.toml` metadata | not published; `cargo publish` would work as-is |
| self-hosted pacman repo | a signed `afanctl.db` next to the release packages | not built; the only option that would give `pacman -Sy` upgrades |

Nothing depends on a channel that is not live: the installer reads the release
assets (see below), and the AUR/Omarchy recipes each build from the tagged
tarball rather than from each other.

## The installer — the one-command path

`install.sh` in the repository root, attached to every release:

```sh
curl -fsSL https://github.com/yadav-prakhar/afanctl/releases/latest/download/install.sh | sudo bash
```

It fetches `SHA256SUMS` from the same release, picks the `x86_64` package line
out of it, downloads that asset, verifies the SHA-256 (a mismatch aborts before
pacman runs) and installs it with `pacman -U`. `--dry-run` resolves, downloads
and verifies without installing; `--version v0.1.0` pins a release. It never
enables or starts the unit — that stays an explicit user decision.

Two properties worth keeping if you edit it:

- **Asset names are not an interface.** The script learns the package filename
  from `SHA256SUMS`, so renaming artifacts in the workflow cannot break an
  installer that is already published.
- **Verification is mandatory** — there is no `--skip-verify`, and the failure
  path exits non-zero *before* `pacman -U`.

## Cutting a release

```sh
# 1. bump the version in Cargo.toml, packaging/PKGBUILD and CHANGELOG.md, commit
# 2. tag: the workflow runs the gates in a clean Arch container, builds the
#    package with packaging/PKGBUILD, cuts the tarball and publishes everything
git tag -a v0.1.1 -m "afanctl 0.1.1" && git push origin v0.1.1
```

The tag, `Cargo.toml` and `packaging/PKGBUILD` are cross-checked, so a mistyped
tag fails the release instead of shipping. `releases/latest/download/install.sh`
follows the newest release, so no step is needed for users to get it.

If AUR registration reopens (or you have a key that is already registered), the
AUR copy is one command away — see below.

## Two PKGBUILDs

| File | Builds from | Used by |
|---|---|---|
| `packaging/PKGBUILD` | your working tree (`--manifest-path ../Cargo.toml`) | development, and the release workflow's artifact |
| `packaging/aur/PKGBUILD` | the source tarball attached to a tagged release | AUR users, if it is ever published |

Both install the same five files (binary, unit, `/etc` config, pristine default
copy, polkit rule) — when you change what the package installs, change both, and
let `makepkg -f` + `pacman -Qlp` on both artifacts be the check that they agree.

## AUR — prepared, not published

The recipe is complete and builds; what is missing is an account. The AUR only
accepts pushes signed by a key registered in an AUR profile, and registration
is currently closed to new maintainers, so this stays on the shelf until that
changes or until someone with an account adopts it.

```sh
# 0. an AUR account with the pubkey below pasted into
#    https://aur.archlinux.org/account (My Account → SSH Public Key)
ssh-keygen -t ed25519 -f ~/.ssh/aur -C aur && cat ~/.ssh/aur.pub
cat >> ~/.ssh/config <<'EOF'
Host aur.archlinux.org
  User aur
  IdentityFile ~/.ssh/aur
EOF

# 1. a git repo is all the AUR needs; `afanctl` already exists in ~/Work/aur/afanctl
#    from the 0.1.0 staging (PKGBUILD, .SRCINFO, LICENSE, REUSE.toml, remote set)
git -C ~/Work/aur/afanctl push origin master

# 2. for later releases: point the recipe at the new tarball and push
packaging/aur/update-aur.sh 0.1.1 --aur-dir ~/Work/aur/afanctl --push
```

`ssh -T aur@aur.archlinux.org` answers `Permission denied (publickey)` until the
pubkey is registered, and says so silently-but-successfully afterwards. Check
the server's identity the first time: the AUR's Ed25519 host key is
`SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4`
(`ssh-keyscan -t ed25519 aur.archlinux.org | ssh-keygen -lf -`).

`update-aur.sh` reads `url=` out of `packaging/aur/PKGBUILD` rather than
inventing a URL, so the release path and the recipe cannot drift.

### Why `packaging/aur` is export-ignored

An AUR `PKGBUILD` publishes a checksum of a release tarball generated from the
tagged tree — so if that tarball contained the PKGBUILD, the checksum would have
to describe a file that contains the checksum. `.gitattributes` keeps
`packaging/aur` out of the tarball (GitHub's own `archive/refs/tags/...` honours
`export-ignore` too), which is why the in-tree `sha256sums` is refreshed by
`update-aur.sh` after a tag rather than before it.

## Omarchy

Omarchy's repository owns its recipes: a package is added as
`pkgbuilds/<name>/` under [omacom/omarchy-pkgs](https://github.com/omacom/omarchy-pkgs),
every recipe carries an `upstream.watch` so their `bin/sync-upstream` follows
releases on its own, and `bin/add-package --source aur` is only a one-time
import. Getting in is a pull request they merge — nothing this repository does
can force it, and nothing here breaks if it is declined.

[omacom/omarchy-pkgs#476](https://github.com/omacom/omarchy-pkgs/pull/476) adds
`afanctl` sourced from the **tagged GitHub tarball**
(`archive/refs/tags/v$pkgver.tar.gz`, which respects the `export-ignore` above)
with a `github` watch on `yadav-prakhar/afanctl` — deliberately independent of
the AUR, so the closed registration does not block it. Their recipe is
authoritative once merged; this repository does not mirror it, precisely so the
two cannot drift.

## Verifying a package without installing it

```sh
(cd packaging     && makepkg -f)                        # build from the working tree
(cd packaging/aur && makepkg -f)                        # build from the released tarball
(cd packaging/aur && makepkg --printsrcinfo > .SRCINFO) # AUR metadata reads back
pacman -Qip afanctl-0.1.0-1-x86_64.pkg.tar.zst          # metadata, deps
pacman -Qlp afanctl-0.1.0-1-x86_64.pkg.tar.zst          # installed file list
```

`makepkg -si` (install) is the user-gated acceptance step in `PLAN.md` §9.4 and
must be run by a human on the target machine — as must `install.sh`, which is
the same install through a different door.
