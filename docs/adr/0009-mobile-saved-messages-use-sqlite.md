---
status: proposed
---

# Mobile saves verified conversation history in SQLite

Previously opened channels should be readable on a cold start, including
offline. The first slice adds a disk preview of the newest channel page under
the existing timeline. A successful relay load replaces that preview. The
same database can address a thread by channel and root event ID.

Each community and signing identity has a versioned SQLite file. iOS uses
the app group's cache directory, which also gives future extension writers a
shared location. The Flutter app uses pinned `sqlite3` 3.9.1 and verifies the
downloaded native binaries against the package's SHA-256 hashes. It runs
SQLite, decoding and signature verification in a background isolate.

The store contains replaceable copies of verified relay events. The app
checks the canonical ID and signature. Relay-only kinds also have to match
the configured relay's NIP-11 `self`. The SQLite file and its sidecars have
a bounded disk budget. The least recently opened conversation pages are
evicted as the cache fills. A corrupt or incompatible file can be deleted
and refilled on demand. Drafts and unsent messages use separate storage.

On iOS, a short background assertion covers each lock-capable operation.
The app stops admitting cache work when the scene resigns active and closes
the database before releasing the assertion. Short transactions and native
cancellation bound the work that can remain at suspension.

## Follow-up

Extend the same store to previously opened threads. When channel lists,
profiles, thread interests, community icons and the open-channel directory
move over, remove each old persisted value after its reader switches. The
directory's old snapshots omitted signatures, so its replacement will
refill from the relay.
