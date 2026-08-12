# Chunk 01 — base-image

**What it does.** Downloads Ubuntu 24.04 (noble) cloud image as an `.ova` into `~/vm-images/`,
verifies it against Ubuntu's published `SHA256SUMS`, and does nothing at all if a valid copy is
already there.

**SOP steps covered:** 1. Rationale lives there, not here.

## Preconditions

- Chunk 00 passes (it checks free disk, which this needs ~600 MB of).
- Network access to `cloud-images.ubuntu.com`.

## Run

```bash
./deploy run 01
```

Force a re-download of an existing image:

```bash
./virtual-box/fetch-image.sh --force
```

## Verify

```bash
ls -lh ~/vm-images/noble.ova
```

Expect a file of roughly 570 MB. Re-running the chunk is the other check — a valid image reports
`already present … nothing to do` and exits 0.

Measured on 2026-08-12: 593,510,400 bytes.

## Rollback

```bash
rm ~/vm-images/noble.ova
```

Nothing else depends on it once chunk 02 has built the VM — the VM holds its own converted copy of
the disk. Deleting it only costs a re-download if you rebuild.

## Traps

- **`.ova`, not `.img`.** Ubuntu's cloud `.img` is QCOW2, which VirtualBox cannot read. Only the
  `.ova` imports natively.
- **A truncated download looks like a valid file.** The script downloads to `noble.ova.part` and moves
  it into place only after a size and checksum check, so an interrupted transfer never leaves something
  that later fails deep inside `VBoxManage import` with an unrelated-looking error.
- **A checksum mismatch is fatal; a missing checksum file is not.** If `SHA256SUMS` cannot be fetched
  the script warns and proceeds, because a size-plausible image plus a clear VM-build failure beats
  refusing to continue over a network hiccup. A *mismatch* always discards the download.
- **`stat` flags differ between macOS and Linux** (`-f %z` vs `-c %s`). The script tries both; a
  hardcoded GNU form silently returns 0 and makes every size check pass.
