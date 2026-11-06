import { assetPrefix } from "./product-branding.mjs";
import { readFileSync, statSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { createMacManifest } from "./create-macos-update-manifest.mjs";
import { verifyRetainedWindows } from "./verify-retained-windows.mjs";

export function assertNewerRelease(tag, latestTag) {
  const parse = (value) => {
    if (!/^v\d+\.\d+\.\d+$/.test(value))
      throw new Error("Expected a stable version tag.");
    const parts = value.slice(1).split(".").map(Number);
    if (!parts.every(Number.isSafeInteger))
      throw new Error("Invalid release version.");
    return parts;
  };
  const candidate = parse(tag);
  // A repository without a published release yet compares against nothing.
  if (!latestTag.trim()) return;
  const latest = parse(latestTag.startsWith("v") ? latestTag : `v${latestTag}`);
  for (let index = 0; index < 3; index++) {
    if (candidate[index] > latest[index]) return;
    if (candidate[index] < latest[index]) break;
  }
  throw new Error(
    "A same or newer release already exists; do not move update channels backward.",
  );
}

export function verifyRelease(
  directory,
  tag,
  root = resolve(import.meta.dirname, ".."),
) {
  const version = JSON.parse(
    readFileSync(resolve(root, "package.json")),
  ).version;
  if (tag !== `v${version}`)
    throw new Error("Release tag must match the app version.");
  if (verifyRetainedWindows(directory, root) !== version)
    throw new Error("Windows and macOS must publish the same version.");
  const trust = JSON.parse(
    readFileSync(resolve(root, "src-tauri/trusted-signers.macos.json")),
  );
  const expected = createMacManifest({ directory, version, tag, trust });
  const manifest = JSON.parse(
    readFileSync(resolve(directory, `updates-macos-epoch-${trust.epoch}.json`)),
  );
  if (
    manifest.version !== version ||
    JSON.stringify(manifest.platforms) !== JSON.stringify(expected.platforms)
  )
    throw new Error(
      "macOS channel does not match the verified release archives.",
    );
  for (const arch of ["aarch64"]) {
    if (!expected.platforms[`darwin-${arch}`])
      throw new Error(`Missing macOS architecture: ${arch}`);
    for (const extension of ["dmg", "zip"]) {
      if (
        !statSync(
          resolve(
            directory,
            `${assetPrefix(version)}_${version}_${arch}.${extension}`,
          ),
        ).size
      )
        throw new Error("An installer is empty.");
    }
  }
  return version;
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
)
  console.log(
    `Verified complete Windows and macOS release ${verifyRelease(resolve(process.argv[2]), process.argv[3])}.`,
  );
