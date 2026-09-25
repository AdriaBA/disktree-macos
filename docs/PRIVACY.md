# Privacy and permissions

Disktree needs to read filesystem metadata to show where disk space is used.
macOS protects many folders even from applications running under your account.

## Full Disk Access

Full Disk Access allows Disktree to inspect protected locations such as data
belonging to Mail, Messages, Safari, other sandboxed applications, and the
Trash. Without it, the app marks those locations as unreadable.

Granting this permission does not make files eligible for automatic deletion.
Disktree still requires you to mark paths, review the complete list, and choose
a removal mode.

To enable it:

1. Open **System Settings → Privacy & Security → Full Disk Access**.
2. Add or enable `disktree.app`.
3. Quit and reopen Disktree.

You can revoke access from the same panel at any time.

## Files and folders prompts

macOS may separately request access to Desktop, Documents, Downloads, removable
volumes, or network volumes. You can deny any permission; the affected location
will simply be unavailable or incomplete in the scan.

## Cloud files

Disktree skips cloud-only files and folders exposed by iCloud Drive, Dropbox,
and other File Provider services. A disk scan should not trigger a large cloud
download.

## Network access

Disktree 0.10.0 scans and processes filesystem metadata on your Mac. It has no
telemetry or network client and does not upload file paths or scan results.

## Removal

- Marking a tile changes only Disktree's in-memory review list.
- The native macOS Trash is the default removal method.
- Permanent deletion is separate and requires explicit confirmation.
- The root, home directory, system trees, mount points, and paths outside the
  scanned root are refused.
- Symlinks are removed as links and are not followed to their targets.

## Source and provenance

The complete source and Git history are available in this repository and at
[the upstream project](https://github.com/tobi/disktree). Mirrored binaries are
published with SHA-256 checksums and linked to the exact upstream release from
which they came.
