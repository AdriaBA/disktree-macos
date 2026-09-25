# Install Disktree on macOS

## Requirements

- macOS 11 or newer
- An Apple Silicon or Intel Mac
- About 10 MB of free space after installation

## 1. Choose the correct download

Open Apple menu → **About This Mac**.

- **Chip: Apple M1, M2, M3, M4, M5 or newer** → download
  `disktree-0.10.0-aarch64-macos.zip`.
- **Processor: Intel…** → download
  `disktree-0.10.0-x86_64-macos.zip`.

About This Mac is the reliable check. A Terminal running through Rosetta can
report `x86_64` on Apple Silicon. If needed, this command prints `1` on an
Apple Silicon Mac:

```sh
sysctl -n hw.optional.arm64
```

Download from the
[latest release](https://github.com/AdriaBA/disktree-macos/releases/latest).

## 2. Verify the download (recommended)

Download the matching `.sha256` file next to the ZIP, open Terminal in that
folder, and run:

```sh
shasum -a 256 -c disktree-0.10.0-aarch64-macos.zip.sha256
```

Use the Intel filename instead when appropriate. A valid download prints
`OK`. Do not open a file whose checksum does not match.

## 3. Install the application

1. Double-click the ZIP to expand it.
2. Drag `disktree.app` into `/Applications` or `~/Applications`.
3. Open it from Applications, Launchpad, or Spotlight.

The app is self-contained. It does not need a shell launcher or a separate
binary elsewhere on your Mac.

## 4. First-launch security prompt

The mirrored v0.10.0 builds are ad-hoc signed. They do not have a Developer ID
signature or an Apple notarization ticket, so Gatekeeper blocks the first
launch.

Try opening the app once. Then go to:

**System Settings → Privacy & Security → Open Anyway**

macOS asks for Touch ID or an administrator password before it opens the app.

Confirm only if the app came from this repository and its SHA-256 checksum
matched. Never disable Gatekeeper globally, and do not run broad quarantine
removal commands copied from the internet.

## 5. Full Disk Access

For a complete disk view:

1. Open **System Settings → Privacy & Security → Full Disk Access**.
2. Add or enable `disktree.app`.
3. Quit and reopen Disktree.

Without this permission, protected folders remain unreadable. The app continues
to work and reports them rather than guessing their contents.

## Update

1. Quit Disktree.
2. Download and verify the newer ZIP.
3. Replace the existing `disktree.app` in Applications.

The bundle identifier remains stable so macOS can retain permissions between
updates.

## Uninstall

Quit Disktree, then move `disktree.app` from Applications to the Trash.

If you built from source with `make install`, run this from the source checkout:

```sh
make uninstall
```

## Troubleshooting

### The app opens but much of `~/Library` is unreadable

Grant Full Disk Access and reopen it.

### macOS says the app cannot be opened

Confirm that you downloaded the correct architecture, verify the checksum, and
use **Privacy & Security → Open Anyway** when offered.

### The app is slow on its first complete scan

A full startup-disk scan can contain millions of files. Start with the home
folder and use `g` only when you need the complete disk.

### Finder and Disktree show different free-space totals

Finder includes purgeable space. Disktree reports the same conservative
filesystem free-space figure that `df` reports.
