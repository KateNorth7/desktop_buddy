# Public download landing page

Public URL: https://katenorth7.github.io/desktop_buddy/ (served from the fork's [`gh-pages` branch](https://github.com/KateNorth7/desktop_buddy/tree/gh-pages) once Pages is enabled there; see below).

**Fork move, 15 November 2026.** Releases publish from `KateNorth7/desktop_buddy` since 0.50.0, but the original site at https://artmarketvm.github.io/desktop_buddy/ still downloaded the frozen v0.49.4 and served the 18 October policy (Nebius/Tavily). The fork's `gh-pages` (`ac15a33`) now looks up releases in the fork and carries the current policy with fork links; the app's `PRIVACY_POLICY_URL`, README, UPDATES.md and `npm run verify:privacy` use the fork's Pages URL. Enabling Pages (Settings → Pages → Deploy from branch `gh-pages` `/`) and Issues on the fork needs a repository admin. The original site is not changed: it belongs to the frozen submission. History below describes the original site.

The owner authorized GitHub Pages publication on 30 October 2026. The existing Pages source was changed from `main` at `/` to the dedicated [`gh-pages` branch](https://github.com/ArtmarketVM/desktop_buddy/tree/gh-pages) at `/`. The application root index.html remains unchanged. Never merge the website branch's root files into app main.

## Content and behavior

- The app’s pixel **sumi** wordmark (bundled Silkscreen, weight 400, 0.08em letter spacing), the owner’s English slogan, the official [YouTube demo](https://www.youtube.com/watch?v=4Vu0HkIB1gY) and two platform download buttons. The owner’s follow-up removed the header mascot/stretching animation.
- Instrument Sans, the app's Cloud/Lime colors and the owner's requested sky-blue highlights; responsive layout, keyboard focus and reduced-motion behavior.
- A single public GitHub latest-release metadata request resolves the exact matching Apple Silicon DMG and Windows x64 EXE. It accepts only uploaded, nonempty, complete pairs from the exact public repository's stable release. Future releases update automatically without editing the page.
- If metadata/JavaScript is unavailable or GitHub rate-limits the request, buttons open the latest release page. The metadata request times out after eight seconds. No API keys, personal accounts, backend, Supabase access, analytics or browser storage are required by the page.
- The small line below the buttons shows only the resolved version. GitHub and Privacy links remain beneath it.

## Official YouTube video

The page embeds [Meet Sumi — Your AI Desktop Companion](https://www.youtube.com/watch?v=4Vu0HkIB1gY), ID `4Vu0HkIB1gY`. Edit `video.youtubeId` in [site-config.mjs on gh-pages](https://github.com/ArtmarketVM/desktop_buddy/blob/gh-pages/site-config.mjs) for a future replacement. Also update the static Watch on YouTube fallback and local poster if the ID changes.

Clicking Play creates the privacy-enhanced YouTube iframe inside the existing responsive 16:9 player, retaining its previous width, rounding and spacing. The ordinary Watch on YouTube link remains available. Before Play, only the locally hosted official thumbnail is loaded. No local MP4, HTML video source or local-video fallback remains.

At the owner's explicit request, the temporary MP4 and its derived poster were removed from the site, the dedicated gh-pages branch was rebuilt with one clean root commit, and four old Pages artifacts containing the MP4 were deleted after the replacement passed deployment. The app's main history, releases and installers were preserved. Old public video/poster URLs return HTTP 404; privacy still matches main. Unreachable Git objects or historical commit URLs can remain in GitHub-controlled caches independently of branch cleanup; do not claim provider-wide physical erasure.

## Privacy and publishing

The existing application policy URL **/desktop_buddy/docs/privacy.html** is preserved with an exact copy of main's docs/privacy.html and its font assets. When the policy changes on main, copy it and any referenced fonts to the same paths on gh-pages, publish, then run `npm run verify:privacy` from the application checkout. Website publication must not silently change the application's configured policy URL.

Commit and push website-only edits to gh-pages. GitHub's built-in Pages deployment handles publication; no desktop CI or installer build is triggered by that branch. Source tests are `node --test tests/site.test.mjs` from the website checkout. Preview via a local static server, then verify YouTube playback, download targets, a mobile viewport and the retained policy URL. The website README contains the source and third-party asset details.

Current website source: `dc608c55302d9a36f4c0139fe4ba76d61b3a1141`, application/privacy/animation source `ab48f00137d119400b961a58b6f643acddfbe60d`, latest public app v0.49.4. Website changes do not bump the desktop app version or alter signing/update trust.
