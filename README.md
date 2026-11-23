# Sumi GitHub Pages

Public landing page: https://katenorth7.github.io/desktop_buddy/

This dedicated `gh-pages` branch hosts the website only. Application source and releases stay on `main`. GitHub Pages publishes this branch's root using `.nojekyll`; no backend, private API key, database, app build or installer update is needed.

## Current video

The official public video is [Meet Sumi — Your AI Desktop Companion](https://www.youtube.com/watch?v=4Vu0HkIB1gY). Its eleven-character ID lives in `site-config.mjs`. The local poster is the video's official YouTube thumbnail. Only clicking Play loads a privacy-enhanced YouTube iframe; before that there is no YouTube iframe or remote thumbnail request. An ordinary Watch on YouTube link works even if JavaScript or embedded playback is unavailable.

No MP4 or locally hosted video fallback remains. The temporary video's asset and derived poster were removed, and the dedicated website branch history was rebuilt at the owner's request. The main application history and releases are unaffected. GitHub may retain unreachable objects or cached historic commit URLs independently; removing a branch reference does not prove physical erasure of every provider-side copy.

To change the video, edit `video.youtubeId`, replace the local poster if needed and update the static Watch on YouTube URL/Open Graph image. Version the changed module/style URLs so returning visitors receive updated assets. Then test and push this branch.

## Downloads and design

- Both platform buttons resolve the latest stable GitHub release's complete uploaded Apple Silicon DMG and Windows x64 EXE pair. Accepted URLs must belong to this exact repository. One bounded public metadata request runs per visit; no polling, browser storage or credentials are used. If JavaScript/API access fails or is rate-limited, ordinary links open the latest GitHub release.
- The header uses the app's actual pixel wordmark: lowercase `sumi` rendered in bundled Silkscreen, weight 400 and 0.08em tracking. The owner-approved English slogan, Instrument Sans, Cloud/Lime palette and version-only line remain. The header mascot was removed at the owner's request. Both download buttons use Lime #E4FA8E on hover; keyboard focus retains the sky-blue outline.
- `docs/privacy.html` and referenced fonts are exact copies from application main, preserving `/desktop_buddy/docs/privacy.html`. When the policy changes, sync both branches, publish and run the app's `npm run verify:privacy`.

## Edit and publish

1. Check out `gh-pages` separately. Never merge its root index.html into the desktop app's main.
2. Run `node --test tests/site.test.mjs`; preview via `python3 -m http.server 1499 --bind 127.0.0.1`.
3. Validate the video link/player, download destinations and retained privacy URL. Commit with an English title and push gh-pages. Website edits do not bump the app version or build installers.

Releases come from `KateNorth7/desktop_buddy` (`REPOSITORY` in `releases.mjs`; from v0.50.0, the frozen hackathon submission ArtmarketVM/desktop_buddy stays at v0.49.4). Download lookup follows future stable releases automatically: publishing a release there updates the buttons and the version line without a site deploy. Intel/universal Mac assets are unsupported. The site has no accounts, forms, AI credentials or Supabase configuration.

## Analytics

Google Analytics 4 loads only when `analytics.measurementId` in `site-config.mjs` holds a valid `G-…` ID; an empty ID publishes the page without any analytics request. Besides page views, the page sends `download` (platform, resolved app version), `play_video`, `watch_on_youtube`, `open_github` and `open_privacy` events. GA4 sets cookies on the visitor's browser; a consent notice for visitors in regions that require one is not part of this page.

Font assets retain their OFL licenses. Platform icons are from [Material Design Icons](https://github.com/Templarian/MaterialDesign), with the included license in assets/platform-icons-LICENSE.txt; the platform marks identify compatibility, not endorsement. The favicon uses the application's existing companion body pattern and Lime preset. Website code is MIT-licensed.
