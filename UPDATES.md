# Application updates

sumi uses signed GitHub Releases for Windows x64 and Apple Silicon macOS 14.5+. Repository commits, tags and ordinary CI artifacts are not delivered application updates. Download the ready-made installers from [Releases](https://github.com/KateNorth7/desktop_buddy/releases/latest); see [installation](docs/JUDGES_START_HERE.md).

## User update flow

Startup checks for a newer published release before workspace editing. A successful mandatory update installs the signature-verified package and restarts the app. On failure/offline operation, the installed app remains available with a retry state. Background and manual checks are handled by the existing updater; they do not rebuild the application.

The Windows installer and macOS updater archive use independent pinned updater trust. macOS app signing is another boundary: releases preserve the reviewed internal certificate and immutable credential helper. Internal signing is not Apple Developer ID/notarization or Windows Authenticode reputation. Never replace trusted keys to bypass verification.

Existing local goals, conversations, memory and settings retain their storage identity. Save drafts before an installation restart. A local candidate below the latest release may update itself on launch; do not install unsigned CI validation artifacts over the internally signed app.

## Maintainer release flow

1. Prepare the final source, release notes and all five synchronized versions before CI. A patch is for fixes; a minor release is for features.
2. Commit/push only with owner authorization. Dispatch [Publish Windows and macOS release](.github/workflows/release.yml) on reviewed `main` with its new version tag.
3. The shared platform/visual workflows reuse only valid content-bound evidence, then build and verify the two signed packages. Publication waits for both platforms.
4. Use `npm run ci:watch -- RUN_ID` once. Inspect detailed logs for a failure or specific diagnostic need.
5. Verify public assets, checksums and update channels once. Keep delivered releases and compatibility bridges immutable.

Pushing a `v*` tag alone does not trigger publication. The obsolete `build-windows.yml` workflow is not an entry point. Current commands, cache/proof rules and required credentials are in [release process](docs/RELEASE_PROCESS.md) and [shared signing](docs/RELEASING_MACOS.md).

## Local development

Windows: `./scripts/dev.ps1` runs development; `./scripts/dev.ps1 -Task build` creates an unsigned personal development installer and does not publish. A local signed Windows package requires an already trusted updater signer. macOS uses the reviewed internal identity; `npm run build:macos` additionally requires the existing macOS updater signing credential. See [macOS](docs/MACOS.md) and [team signing](docs/TEAM_SIGNING.md).

Provider keys are unrelated to updater signing. Built-in AI uses server-side keys; optional personal keys stay runtime-only in the OS vault or local process environment. No private signing material or provider key belongs in Git, frontend bundles, release assets or logs.
