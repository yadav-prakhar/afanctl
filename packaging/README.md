# Publishing afanctl

How a release becomes installable — from `yay -S afanctl`, from a released
`.pkg.tar.zst`, and from Omarchy's package repository.

## What ships where

| Channel | Artifact | Maintained by |
| ------- | -------- | ------------- |
| GitHub release | source tarball, `afanctl-<ver>-1-x86_64.pkg.tar.zst`, `SHA256SUMS` | `.github/workflows/release.yml` (tag push) |
| AUR (`afanctl`) | this directory's `PKGBUILD` + `.SRCINFO` | `update-aur.sh` |
| Omarchy Package Repository (`pkgs.omarchy.org`) | a recipe they own, watching our tagged releases | [omacom/omarchy-pkgs#476](https://github.com/omacom/omarchy-pkgs/pull/476) — their merge decides it |

`packaging/PKGBUILD` (one level up) is the *development* PKGBUILD: it builds the
package from a working tree. It is not published anywhere. This directory is the
published one, and it builds from the released source tarball. Both install the
same five files — when you change what the package installs, change both.

## Cutting a release

```sh
# 1. version bump + changelog, then commit
#    Cargo.toml version, CHANGELOG.md

# 2. tag — the release workflow builds the artifacts and opens the release
git tag -a v0.1.1 -m "afanctl 0.1.1" && git push origin v0.1.1

# 3. point the AUR package at the new tarball (downloads the asset, rewrites
#    pkgver + sha256sums here, mirrors into the AUR clone, commits)
packaging/aur/update-aur.sh 0.1.1 --aur-dir ~/Work/aur/afanctl

# 4. push the checksum commit here, and the AUR commit there
git commit -am "packaging: AUR checksums for v0.1.1" && git push
git -C ~/Work/aur/afanctl push origin master
```

Step 3 needs an AUR clone; step 4 needs an SSH key registered in your AUR
account. See *First-time AUR setup* below.

## First-time AUR setup

The AUR takes pushes over SSH only, and the key must be listed in your AUR
account, so this part is yours to do once:

```sh
# 0. create the account: https://aur.archlinux.org/register (email verification),
#    then confirm the address from the link it sends.

# 1. a key pair used for nothing else (AUR recommends a dedicated one)
ssh-keygen -t ed25519 -f ~/.ssh/aur -C aur

# 2. paste the PUBKEY into https://aur.archlinux.org/account
#    (My Account → SSH Public Key)
cat ~/.ssh/aur.pub

# 3. tell ssh to use it for the AUR
cat >> ~/.ssh/config <<'EOF'
Host aur.archlinux.org
  User aur
  IdentityFile ~/.ssh/aur
EOF

# 4. clone, point it at the release, push
git -c init.defaultBranch=master clone ssh://aur@aur.archlinux.org/afanctl.git ~/Work/aur/afanctl
packaging/aur/update-aur.sh 0.1.0 --aur-dir ~/Work/aur/afanctl --push
```

Check the auth before blaming the package — `ssh -T aur@aur.archlinux.org` is a
`Permission denied (publickey)` until the pubkey is registered, and works
silently afterwards. Verify the server's key fingerprint the first time: the
AUR's Ed25519 host key is
`SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4`
(`ssh-keyscan -t ed25519 aur.archlinux.org | ssh-keygen -lf -`).

`update-aur.sh` refuses to invent metadata: it reads `url=` from this
directory's PKGBUILD, so the release URL and the AUR package can never drift.

## Why the checksums are updated after the tag

The AUR `PKGBUILD` publishes a checksum of the release tarball, and that tarball
is generated from the tagged tree. `.gitattributes` export-ignores
`packaging/aur`, so the tarball never contains this directory — otherwise the
checksum would have to describe a file that contains the checksum. The cost is
that the in-tree `sha256sums` is one release behind until step 3 runs; the AUR
copy is never stale, because step 3 writes it there.

## Omarchy

The Omarchy Package Repository owns its own recipes: a package is added as
`pkgbuilds/<name>/` under [omacom/omarchy-pkgs](https://github.com/omacom/omarchy-pkgs),
and every recipe carries an `upstream.watch` so their `bin/sync-upstream` picks
up new releases on its own. Getting in is a pull request they merge — nothing
this repository does can force it.

That request is open as
[omacom/omarchy-pkgs#476](https://github.com/omacom/omarchy-pkgs/pull/476). It
adds `afanctl` sourced from the **tagged GitHub tarball**
(`archive/refs/tags/v$pkgver.tar.gz`, which respects the `export-ignore` in
`.gitattributes`, so their build never sees this directory) with a `github`
watch on `yadav-prakhar/afanctl`. Their recipe is authoritative once merged —
this repository does not mirror it, precisely so the two cannot drift.

If the request is declined, nothing here breaks: the AUR package and the
release artifacts stand on their own.

## Verifying a package without installing it

```sh
cd packaging/aur && makepkg --printsrcinfo > .SRCINFO   # metadata sanity
makepkg -f                                              # build, no install
namcap afanctl-0.1.0-1-x86_64.pkg.tar.zst               # Arch's linter
```

`makepkg -si` (install) is the user-gated acceptance step in `PLAN.md` §9.4 and
must be run by a human on the target machine.