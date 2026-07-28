# Media Storage

Buzz uses the Blossom protocol for media uploads and storage, backed by S3-compatible object storage (MinIO in development, any S3 provider in production).

- Users paste, drag, or attach files in channels
- Files are uploaded to S3 via the relay's HTTP endpoints (NIP-98 auth)
- The relay generates server-side thumbnails for images
- Files are served with content-type headers and caching
- Media events are Nostr events with references to the stored blob

**Configuration:** S3 endpoint, bucket, region, access key, and secret key via environment variables.

**Related:**
- [buzz-media](../components/buzz-media)
- [Relay](../entities/relay)
