import { assetPrefix, releaseDownloadUrl } from "./product-branding.mjs";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { validateTrust, verifyArtifact } from "./signing-trust.mjs";

export function createMacManifest({ directory, version, tag, trust }) {
  if (!/^\d+\.\d+\.\d+$/.test(version) || tag !== `v${version}`)
    throw new Error("Release tag must match the app version.");
  validateTrust(trust);
  const files = readdirSync(directory);
  if (files.some((name) => /^(?:sumi|Desktop\.Buddy)_.*_x86_64\./.test(name)))
    throw new Error("Intel macOS release assets are not supported.");
  const platforms = {};
  for (const architecture of ["aarch64"]) {
    const name = `${assetPrefix(version)}_${version}_${architecture}.app.tar.gz`;
    if (!files.includes(name)) continue;
    const bytes = readFileSync(resolve(directory, name));
    if (!bytes.length) throw new Error("The updater archive is empty.");
    const signature = readFileSync(
      resolve(directory, `${name}.sig`),
      "utf8",
    ).trim();
    verifyArtifact(bytes, signature, trust, version);
    platforms[`darwin-${architecture}`] = {
      signature,
      url: releaseDownloadUrl(tag, name),
    };
  }
  if (!Object.keys(platforms).length)
    throw new Error("Missing macOS updater archive.");
  return {
    version,
    notes: "",
    pub_date: new Date().toISOString(),
    platforms,
  };
}
if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  const root = resolve(import.meta.dirname, "..");
  const directory = resolve(process.argv[2] ?? "artifacts");
  const version = JSON.parse(
    readFileSync(resolve(root, "package.json")),
  ).version;
  const trust = JSON.parse(
    readFileSync(resolve(root, "src-tauri/trusted-signers.macos.json")),
  );
  const manifest = createMacManifest({
    directory,
    version,
    tag: process.argv[3],
    trust,
  });
  writeFileSync(
    resolve(directory, `updates-macos-epoch-${trust.epoch}.json`),
    JSON.stringify(manifest, null, 2) + "\n",
  );
  console.log(`Verified macOS updater manifest for ${version}.`);
}
