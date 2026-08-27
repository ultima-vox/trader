# Canonical Vox Trader design source

The original uploaded canonical HTML is preserved byte-for-byte as an XZ-compressed, base64-encoded source split into three text parts so it can be stored through the repository text-content path without altering the original bytes.

## Restore

From repository root:

```bash
bash docs/design/source/restore-canonical-design.sh
```

This creates:

```text
docs/design/source/Vox-Trader-Design-System-canonical.html
```

The script verifies both compressed and restored SHA-256 checksums.

### Integrity

Original HTML SHA-256:

```text
5da71028760066f8781af367dc42daa1c65a586e544315947fedf71d8a473196
```

Compressed XZ SHA-256:

```text
928a31ea7d3e1d41f421a4534f3dcc34819b76d7753b1a1f7826352cd3c0832c
```

The split files are transport/storage artifacts only. They are not design-system implementation files.

Read `../CANONICAL_DESIGN.md` before reconciling the repository design system.
