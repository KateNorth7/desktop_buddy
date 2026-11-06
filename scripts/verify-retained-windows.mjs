import { assetPrefix, releaseDownloadUrl } from "./product-branding.mjs";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { readTrust, verifyArtifact } from "./signing-trust.mjs";

export function verifyRetainedWindows(
  directory,
  root = resolve(import.meta.dirname, ".."),
) {
  const trust = readTrust(root);
  const read = (name) =>
    JSON.parse(readFileSync(resolve(directory, name), "utf8"));
  const compatibility = JSON.parse(
    readFileSync(resolve(root, "updates/compatibility.json"), "utf8"),
  );
  for (const [name, expected] of Object.entries(compatibility)) {
    if (JSON.stringify(read(name)) !== JSON.stringify(expected))
      throw new Error(`Windows compatibility channel changed: ${name}`);
  }
  const manifest = read(`updates-epoch-${trust.epoch}.json`);
  const platform = manifest.platforms?.["windows-x86_64"];
  if (!/^\d+\.\d+\.\d+$/.test(manifest.version) || !platform)
    throw new Error("Invalid retained Windows manifest.");
  const name = `${assetPrefix(manifest.version)}_${manifest.version}_x64-setup.exe`;
  const expectedUrl = releaseDownloadUrl(`v${manifest.version}`, name);
  if (platform.url !== expectedUrl)
    throw new Error("Windows updater must keep its pinned official URL.");
  const signature = readFileSync(
    resolve(directory, `${name}.sig`),
    "utf8",
  ).trim();
  if (signature !== platform.signature)
    throw new Error("Windows asset and manifest signatures differ.");
  verifyArtifact(
    readFileSync(resolve(directory, name)),
    signature,
    trust,
    manifest.version,
  );
  return manifest.version;
}
if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
)
  console.log(
    `Verified retained Windows release ${verifyRetainedWindows(resolve(process.argv[2]))}.`,
  );
