# buzz-media

Blossom protocol media storage backed by S3-compatible object storage.

**Key responsibilities:**
- File upload via HTTP (NIP-98 auth)
- Thumbnail generation (server-side image processing)
- File serving with content-type headers and caching
- Blob deletion
- S3 configuration (endpoint, bucket, region, credentials)

**Related:**
- [MediaStorage](../concepts/media-storage)
- [buzz-relay](buzz-relay) — serves media endpoints
