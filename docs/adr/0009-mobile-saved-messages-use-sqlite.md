---
status: proposed
---

# Mobile saves verified conversation history in SQLite

Previously opened channels should be readable on a cold start, including
offline. The first slice adds a disk preview of the newest channel page under
the existing timeline. A successful relay load replaces that preview. The
same database can address a thread by channel and root event ID.

Each community and signing identity has a versioned SQLite file in the
app's private cache directory on iOS and Android. The Flutter app uses pinned
`sqlite3` 3.9.1 and verifies the downloaded native binaries against the
package's SHA-256 hashes. It runs SQLite, decoding and signature verification
in a background isolate.

The store contains replaceable copies of verified relay events. The app
checks the canonical ID and signature. Relay-only kinds also have to match
the configured relay's NIP-11 `self`. The SQLite file and its sidecars have
a bounded disk budget. The least recently opened conversation pages are
evicted as the cache fills. If indexes and references fill the SQLite file
before the wire budget is reached, the store evicts older pages and retries
the save once. The app discards corrupt or incompatible files and refills
them on demand. The cache requires no migration. Drafts and unsent messages
use separate storage.

Only the app opens the database. A shared SQLite file would require the app
and extensions to coordinate access through suspension. iOS can terminate a
process suspended with a lock on a shared database. The app uses short WAL
transactions and closes the connection after each operation. If a future
extension needs to contribute, it can hand the app a small file to verify
and import. [Apple TN2408](https://developer.apple.com/library/archive/technotes/tn2408/_index.html) ·
[GRDB's database sharing guide](https://swiftpackageindex.com/groue/GRDB.swift/documentation/grdb/databasesharing)

Leaving or deleting a channel, an explicit relay access denial, and live
access revocation purge its saved pages. Sign-out, community removal, and
identity replacement purge the affected identity. Startup removes abandoned
and incompatible cache files.

## Follow-up

Extend the same store to previously opened threads. When channel lists,
profiles, thread interests, community icons and the open-channel directory
move over, remove each old persisted value after its reader switches. The
directory's old snapshots omitted signatures, so its replacement will
refill from the relay.
