import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { readTrust, verifyArtifact, validateTrust } from "./signing-trust.mjs";
import { releaseDownloadUrl } from "./product-branding.mjs";

export function createManifest({
  directory,
  version,
  tag,
  notes = "",
  publishedAt = new Date().toISOString(),
}) {
  if (!/^\d+\.\d+\.\d+$/.test(version) || tag !== `v${version}`) {
    throw new Error("The release tag must match the synchronized app version.");
  }
  const installers = readdirSync(directory).filter((name) =>
    name.endsWith(`_${version}_x64-setup.exe`),
  );
  if (
    installers.length !== 1 ||
    !installers[0].endsWith(`_${version}_x64-setup.exe`)
  ) {
    throw new Error(
      "Expected exactly one x64 installer for this release version.",
    );
  }
  const installer = installers[0];
  // GitHub normalizes spaces to periods when naming uploaded release assets.
  const assetName = installer.replaceAll(" ", ".");
  if (readFileSync(resolve(directory, installer)).length === 0)
    throw new Error("The installer is empty.");
  const signature = readFileSync(
    resolve(directory, `${installer}.sig`),
    "utf8",
  ).trim();
  const decoded = Buffer.from(signature, "base64").toString("utf8");
  if (
    !decoded.startsWith("untrusted comment:") ||
    !decoded.includes("trusted comment:")
  ) {
    throw new Error("The installer is missing a Tauri updater signature.");
  }
  return {
    version,
    notes,
    pub_date: publishedAt,
    platforms: {
      "windows-x86_64": {
        signature,
        url: releaseDownloadUrl(tag, encodeURIComponent(assetName)),
      },
    },
  };
}

export function prepareChannels({
  manifest,
  bytes,
  trust,
  history,
  compatibility,
}) {
  validateTrust(trust);
  const signature = manifest.platforms["windows-x86_64"].signature;
  verifyArtifact(bytes, signature, trust, manifest.version);
  const archive = structuredClone(compatibility);
  for (const value of Object.values(archive)) {
    if (compareVersion(value.version, manifest.version) > 0)
      throw new Error(
        "A compatibility bridge cannot be newer than the release.",
      );
    if (
      value.version === manifest.version &&
      value.platforms["windows-x86_64"].signature !== signature
    )
      throw new Error(
        "This bridge version is already archived with a different signature. Do not replace its installer.",
      );
  }
  if (!archive["latest.json"]) {
    if (trust.epoch !== 1)
      throw new Error(
        "Publish and archive the initial legacy bridge before changing trusted signers.",
      );
    // Old installations always receive this signed bridge, even if they skip later releases.
    archive["latest.json"] = manifest;
  }
  for (let epoch = 1; epoch < trust.epoch; epoch++) {
    const name = `updates-epoch-${epoch}.json`;
    if (!archive[name]) {
      if (epoch !== trust.epoch - 1 || !history[String(epoch)])
        throw new Error("An older compatibility channel is missing.");
      // A newly added signer cannot authorize itself. The bridge must use an existing key.
      verifyArtifact(
        bytes,
        signature,
        validateTrust(history[String(epoch)]),
        manifest.version,
      );
      archive[name] = manifest;
    }
    if (compareVersion(archive[name].version, manifest.version) > 0)
      throw new Error(
        "A compatibility bridge cannot be newer than the release.",
      );
  }
  return {
    archive,
    files: { ...archive, [`updates-epoch-${trust.epoch}.json`]: manifest },
  };
}
function compareVersion(a, b) {
  if (![a, b].every((v) => /^\d+\.\d+\.\d+$/.test(v)))
    throw new Error("Invalid bridge version.");
  const left = a.split(".").map(Number),
    right = b.split(".").map(Number);
  for (let i = 0; i < 3; i++)
    if (left[i] !== right[i]) return left[i] - right[i];
  return 0;
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  const root = resolve(import.meta.dirname, "..");
  const version = JSON.parse(
    readFileSync(resolve(root, "package.json"), "utf8"),
  ).version;
  const directory = resolve(root, "src-tauri/target/release/bundle/nsis");
  const manifest = createManifest({
    directory,
    version,
    tag: process.argv[2],
    notes: "",
  });
  const trust = readTrust(root);
  const compatibilityPath = resolve(root, "updates/compatibility.json");
  const channels = prepareChannels({
    manifest,
    bytes: readFileSync(resolve(directory, `sumi_${version}_x64-setup.exe`)),
    trust,
    history: JSON.parse(
      readFileSync(resolve(root, "updates/trust-history.json"), "utf8"),
    ),
    compatibility: JSON.parse(readFileSync(compatibilityPath, "utf8")),
  });
  for (const [name, value] of Object.entries(channels.files))
    writeFileSync(
      resolve(directory, name),
      JSON.stringify(value, null, 2) + "\n",
    );
  writeFileSync(
    compatibilityPath,
    JSON.stringify(channels.archive, null, 2) + "\n",
  );
  console.log(
    `Verified updater manifests created for ${version}, trust epoch ${trust.epoch}. Archive updates/compatibility.json with the release source.`,
  );
}
