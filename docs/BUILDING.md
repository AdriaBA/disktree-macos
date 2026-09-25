# Build, sign, and notarize on macOS

## Requirements

- macOS 11 or newer
- Rust 1.97 or newer, installed with [rustup](https://rustup.rs)
- Xcode or the Xcode Command Line Tools

Check the toolchain:

```sh
rustc --version
xcode-select -p
```

## Build and test

```sh
git clone https://github.com/AdriaBA/disktree-macos.git
cd disktree-macos
cargo xtask lint
cargo xtask test
cargo build --release --locked -p disktree-app
```

The project treats Clippy warnings as errors. Do not publish a build unless the
lint and test gates pass.

## Create a local app bundle

```sh
make bundle
```

This creates:

- `target/bundle/disktree.app`
- a ZIP suitable for transfer

Local bundles are ad-hoc signed unless a Developer ID identity is supplied.
Install the result with:

```sh
make install
```

That installs `~/Applications/disktree.app` and links the command-line entry
under `~/.local/bin/disktree`.

## Developer ID signing and notarization

Public distribution without a Gatekeeper warning requires an Apple Developer
Program membership, a Developer ID Application certificate, and notarization.

Store notary credentials once:

```sh
xcrun notarytool store-credentials <profile-name>
```

Then package with the identity and profile:

```sh
NOTARY_PROFILE=<profile-name> cargo xtask bundle \
  --sign "Developer ID Application: Name (TEAMID)" --notarize
```

The packaging task creates the bundle, signs it, submits it to Apple's notary
service, staples the ticket, and verifies the result. Never commit certificates,
Apple credentials, or notary secrets to the repository.

## Release architecture

Build `aarch64-apple-darwin` on Apple Silicon and `x86_64-apple-darwin` on an
Intel runner. Native GitHub-hosted runners avoid shipping an artifact that was
never exercised on its target architecture.

The upstream release workflow is the source of truth for official builds. This
Mac-focused fork mirrors those artifacts only after their published SHA-256
checksums have been verified.

## Publish a verified mirror

For each upstream version:

1. Download the Apple Silicon and Intel ZIPs and their `.sha256` files from the
   matching upstream release tag.
2. Run `shasum -a 256 -c` against both checksum files.
3. Confirm the binaries are `arm64` and `x86_64`, and that the bundle version
   matches the release tag.
4. Publish all four files under the same tag in this fork. Keep the upstream
   filenames unchanged so the pinned documentation links remain auditable.

The v0.10.0 mirror uses the existing `v0.10.0` source tag and these exact
assets:

```text
disktree-0.10.0-aarch64-macos.zip
disktree-0.10.0-aarch64-macos.zip.sha256
disktree-0.10.0-x86_64-macos.zip
disktree-0.10.0-x86_64-macos.zip.sha256
```
