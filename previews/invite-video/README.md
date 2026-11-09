# Invite preview media

A 16-second silent Buzz animation and wordmark-only PNG fallback, with a browser tool for reviewing motion and choosing poster frames. The animation uses the onboarding wordmark and fuzzy shader, pale-blue/lime bees, diagonal paired flights and a solo loop-de-loop.

## Preview

From the repository root:

```sh
python3 -m http.server 4319 --bind 127.0.0.1 --directory previews/invite-video
```

Open http://127.0.0.1:4319. Scrub or choose a suggested frame, then select **Use this frame**. Poster choices persist only in that browser; **Download poster PNG** exports the choice. The committed poster is the approved wordmark-only frame at 0 seconds. The iMessage-style card is a composition simulation, not evidence of a native Messages unfurl.

## Media

- `buzz-invite-loop.mp4`: 960 × 540, 30 fps, H.264/yuv420p, fast-start, no audio.
- `buzz-invite-poster.png`: 960 × 540, wordmark only.
- `buzz-wordmark.png` and `wordmark.frag`: onboarding artwork and 400 ms grain/displacement shader, from the Buzz desktop onboarding implementation; bee silhouette matches the mobile welcome artwork.
- Background: `#D7D72E`; bees: `#D7E7F5` and `#DBFA52`.

## Verify or regenerate

Activate the repository's Hermit environment first. Install Chrome, Python 3, and ffmpeg. Then:

```sh
cd previews/invite-video
npm ci
npm test
npm run render
```

Verification starts an isolated localhost server and checks poster selection, PNG export, persistence, loop continuity, mobile overflow and reduced-motion startup. Rendering also rebuilds both committed media files. Temporary captures stay in `.output/`. `PREVIEW_BROWSER` may select another installed Playwright Chromium channel.

## Production invite previews

The production web build includes the approved MP4 and wordmark-only poster at
`/assets/invite-preview/buzz-invite-loop-v1.mp4` and
`/assets/invite-preview/buzz-invite-poster-v1.png`. The relay adds Open Graph
metadata to the initial HTML for `/invite/<code>` only. Previews are generic and
do not validate or redeem the invite.

Deploy both the updated relay and its web bundle (the relay Docker image includes
both). The media host comes from the request's resolved community, matching the
invite link's host. `RELAY_URL` supplies only the `wss`/`ws` TLS posture, mapped to
`https`/`http`. Unmapped hosts or lookup failures receive no preview metadata.
Each community host must serve the web bundle without authentication. Use HTTPS
in production.

After deployment, inspect an invite response with `curl` for `og:video` and
`og:image`, then fetch both referenced URLs and check that they return MP4/PNG
content rather than a login page. The video endpoint supports HTTP range
requests. Share a fresh invite link in Messages to test actual playback; receiving
apps decide whether to play video or use the poster, and may cache previews.

When replacing approved media, bump the versioned filenames in both
`web/vite.config.ts` and `crates/buzz-relay/src/invite_preview.rs` to avoid stale
cached media.

The receiving app chooses playback behavior. The 16-second duration is an animation choice, not an asserted platform limit.
