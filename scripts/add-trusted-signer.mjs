import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { publicKey, validateTrust } from "./signing-trust.mjs";
import { updateChannelUrl } from "./product-branding.mjs";

export function addSigner(trust, id, publicFile) {
  const next = {
    epoch: trust.epoch + 1,
    signers: [...trust.signers, { id, pubkey: publicKey(publicFile).encoded }],
  };
  return validateTrust(next);
}
if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  const root = resolve(import.meta.dirname, "..");
  const [id, path] = process.argv.slice(2);
  if (!id || !path)
    throw new Error(
      "Usage: node scripts/add-trusted-signer.mjs signer-id public-key.pub",
    );
  const trustPath = resolve(root, "src-tauri/trusted-signers.json");
  const trust = JSON.parse(readFileSync(trustPath, "utf8"));
  const configPath = resolve(root, "src-tauri/tauri.conf.json");
  const config = JSON.parse(readFileSync(configPath, "utf8"));
  validateTrust(trust, config.plugins.updater.pubkey);
  const next = addSigner(trust, id, readFileSync(resolve(path), "utf8"));
  const historyPath = resolve(root, "updates/trust-history.json");
  const history = JSON.parse(readFileSync(historyPath, "utf8"));
  if (history[String(trust.epoch)])
    throw new Error(
      "This trust epoch already has a successor. Finish its transition release before enrolling another signer.",
    );
  history[String(trust.epoch)] = trust;
  config.plugins.updater.endpoints = [
    updateChannelUrl(`updates-epoch-${next.epoch}.json`),
  ];
  writeFileSync(trustPath, JSON.stringify(next, null, 2) + "\n");
  writeFileSync(configPath, JSON.stringify(config, null, 2) + "\n");
  writeFileSync(historyPath, JSON.stringify(history, null, 2) + "\n");
  console.log(
    `Public signer ${id} enrolled for trust epoch ${next.epoch}. Review and ship a new transition release signed by an already trusted signer. No private key was imported.`,
  );
}
