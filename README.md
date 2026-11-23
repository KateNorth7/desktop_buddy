# Sumi GitHub Pages

Public landing page: https://katenorth7.github.io/desktop_buddy/

This dedicated `gh-pages` branch hosts the website only. Application source and releases stay on `main`. GitHub Pages publishes this branch's root using `.nojekyll`; no backend, private API key, database, app build or installer update is needed.

## Current video

The official public video is [Meet Sumi — Your AI Desktop Companion](https://www.youtube.com/watch?v=4Vu0HkIB1gY). Its eleven-character ID lives in `site-config.mjs`. The local poster is the video's official YouTube thumbnail. Only clicking Play loads a privacy-enhanced YouTube iframe; before that there is no YouTube iframe or remote thumbnail request. An ordinary Watch on YouTube link works even if JavaScript or embedded playback is unavailable.

No MP4 or locally hosted video fallback remains. The temporary video's asset and derived poster were removed, and the dedicated website branch history was rebuilt at the owner's request. The main application history and releases are unaffected. GitHub may retain unreachable objects or cached historic commit URLs independently; removing a branch reference does not prove physical erasure of every provider-side copy.

To change the video, edit `video.youtubeId`, replace the local poster if needed and update the static Watch on YouTube URL/Open Graph image. Version the changed module/style URLs so returning visitors receive updated assets. Then test and push this branch.

## Downloads and design

- Both platform buttons resolve the latest stable GitHub release's complete uploaded Apple Silicon DMG and Windows x64 EXE pair. Accepted URLs must belong to this exact repository. One bounded public metadata request runs per visit; no polling, browser storage or credentials are used. If JavaScript/API access fails or is rate-limited, ordinary links open the latest GitHub release.
- Layout (23 November 2026): a two-column hero (headline, one sentence, two download buttons on the left; the pixel companion with one card on the right), then the demo video, then a one-line footer with the privacy sentence, version, GitHub and Privacy. The header is the app's pixel wordmark: lowercase `sumi` in bundled Silkscreen, weight 400 and 0.08em tracking. Instrument Sans 400/500/600 (600 only for the headline), Cloud/Lime palette; the macOS button is Lime, both turn Lime on hover; keyboard focus keeps the sky-blue outline.
- The companion (`companion.mjs`, `assets/sumi-mascot.png`) shows three cards the app really sends, one at a time: a completion offer, a deadline check-in and a new-work offer. A card waits for a click; the reply shows for five seconds, then the next card appears. The mascot hovers and hops on a click. Nothing is sent or stored; `prefers-reduced-motion` turns the motion off. Clicks are reported to analytics as `companion_reply` (card, choice).
- `docs/privacy.html` and referenced fonts are exact copies from application main, preserving `/desktop_buddy/docs/privacy.html`. When the policy changes, sync both branches, publish and run the app's `npm run verify:privacy`.

## Edit and publish

1. Check out `gh-pages` separately. Never merge its root index.html into the desktop app's main.
2. Run `node --test tests/site.test.mjs`; preview via `python3 -m http.server 1499 --bind 127.0.0.1`.
3. Validate the video link/player, download destinations and retained privacy URL. Commit with an English title and push gh-pages. Website edits do not bump the app version or build installers.

Releases come from `KateNorth7/desktop_buddy` (`REPOSITORY` in `releases.mjs`; from v0.50.0, the frozen hackathon submission ArtmarketVM/desktop_buddy stays at v0.49.4). Download lookup follows future stable releases automatically: publishing a release there updates the buttons and the version line without a site deploy. Intel/universal Mac assets are unsupported. The site has no accounts, forms, AI credentials or Supabase configuration.

## Analytics

Google Analytics 4 loads only when `analytics.measurementId` in `site-config.mjs` holds a valid `G-…` ID; an empty ID publishes the page without any analytics request. Besides page views, the page sends `download` (platform, resolved app version), `play_video`, `companion_reply` (card, choice), `open_github`, `open_privacy` and, from the no-JavaScript fallback link, `watch_on_youtube` events. GA4 sets cookies on the visitor's browser; a consent notice for visitors in regions that require one is not part of this page.

Font assets retain their OFL licenses. Platform icons are from [Material Design Icons](https://github.com/Templarian/MaterialDesign), with the included license in assets/platform-icons-LICENSE.txt; the platform marks identify compatibility, not endorsement. The favicon uses the application's existing companion body pattern and Lime preset. Website code is MIT-licensed.
