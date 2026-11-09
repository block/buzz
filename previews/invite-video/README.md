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

## Integration boundary

This PR prepares media and preview tooling. It does not change live invite HTML or deploy the relay. To enable real link previews, serve the media at public absolute HTTPS URLs and add `og:video` (direct MP4), `og:video:type` (`video/mp4`), dimensions, `og:image` (poster), title and description to the initial invite HTML. Keep generic branding on the public preview and never redeem an invite during a crawler fetch. Ship the relay image and verify an actual shared invite in the target clients.

The receiving app chooses playback behavior. The 16-second duration is an animation choice, not an asserted platform limit.
