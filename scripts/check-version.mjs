import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { readTrust, validateTrust } from "./signing-trust.mjs";
import { updateChannelUrl } from "./product-branding.mjs";
const read = (path) =>
  readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
const version = JSON.parse(read("package.json")).version;
const lock = JSON.parse(read("package-lock.json"));
const cargo = read("src-tauri/Cargo.toml").match(/^version = "([^"]+)"/m)?.[1];
const cargoLock = read("src-tauri/Cargo.lock").match(
  /name = "desktop-buddy"\r?\nversion = "([^"]+)"/,
)?.[1];
const tauri = JSON.parse(read("src-tauri/tauri.conf.json")).version;
if (
  [lock.version, lock.packages[""].version, cargo, cargoLock, tauri].some(
    (v) => v !== version,
  )
) {
  throw new Error(
    "Release versions differ. Update package.json, package-lock.json, Cargo.toml, Cargo.lock and tauri.conf.json together.",
  );
}
console.log(`Release version: ${version}`);
const trust = readTrust(fileURLToPath(new URL("..", import.meta.url)));
const updater = JSON.parse(read("src-tauri/tauri.conf.json")).plugins.updater;
if (
  updater.endpoints[0] !==
    updateChannelUrl(`updates-epoch-${trust.epoch}.json`) ||
  updater.requireSignedVersion !== true
)
  throw new Error(
    "Updater configuration must match the trusted signing epoch and require signed versions.",
  );

const mac = JSON.parse(read("src-tauri/tauri.macos.conf.json"));
const macTrust = validateTrust(
  JSON.parse(read("src-tauri/trusted-signers.macos.json")),
  mac.plugins.updater.pubkey,
);
if (
  mac.plugins.updater.endpoints[0] !==
    updateChannelUrl(`updates-macos-epoch-${macTrust.epoch}.json`) ||
  mac.plugins.updater.requireSignedVersion !== true ||
  mac.bundle.createUpdaterArtifacts !== true
) {
  throw new Error(
    "macOS updater must produce signed artifacts for its trusted channel.",
  );
}
