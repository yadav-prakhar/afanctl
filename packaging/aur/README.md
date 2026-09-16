# Publishing afanctl

How a release becomes installable — from `yay -S afanctl`, from a released
`.pkg.tar.zst`, and from Omarchy's package repository.

## What ships where

| Channel | Artifact | Maintained by |
| ------- | -------- | ------------- |
| GitHub release | source tarball, `afanctl-<ver>-1-x86_64.pkg.tar.zst`, `SHA256SUMS` | `.github/workflows/release.yml` (tag push) |
| AUR (`afanctl`) | this directory's `PKGBUILD` + `.SRCINFO` | `update-aur.sh` |
| Omarchy Package Repository (`pkgs.omarchy.org`) | built from the AUR package | a PR to [omacom/omarchy-pkgs](https://github.com/omacom/omarchy-pkgs) |

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
# 1. a key pair used for nothing else (AUR recommends a dedicated one)
ssh-keygen -t ed25519 -f ~/.ssh/aur -C aur

# 2. add the pubkey to https://aur.archlinux.org/account (My Account → SSH Public Key)
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

Omarchy's package repository builds packages *from this AUR package*: their
`bin/add-package afanctl` copies the AUR PKGBUILD into
`pkgbuilds/afanctl/` with `.omarchy/package.json` recording `{"source": "aur"}`
plus the AUR commit it synced, and their builder then tracks the AUR package
every 6 hours. Nothing ships to `pkgs.omarchy.org` until a maintainer merges
that addition, so publishing to the AUR is the prerequisite — after that it is a
pull request against `omacom/omarchy-pkgs`.

## Verifying a package without installing it

```sh
cd packaging/aur && makepkg --printsrcinfo > .SRCINFO   # metadata sanity
makepkg -f                                              # build, no install
namcap afanctl-0.1.0-1-x86_64.pkg.tar.zst               # Arch's linter
```

`makepkg -si` (install) is the user-gated acceptance step in `PLAN.md` §9.4 and
must be run by a human on the target machine.