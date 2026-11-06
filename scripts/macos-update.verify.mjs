import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createMacManifest } from "./create-macos-update-manifest.mjs";

test("macOS manifests verify the archive, signer and signed version before publishing", () => {
  const fixture = JSON.parse(
    readFileSync(
      new URL("../src-tauri/test-data/signing.json", import.meta.url),
    ),
  );
  const signer = fixture.signers[0];
  const trust = { epoch: 1, signers: [{ id: "macos", pubkey: signer.pubkey }] };
  const directory = mkdtempSync(join(tmpdir(), "buddy-macos-release-"));
  const name = "Desktop.Buddy_0.13.0_aarch64.app.tar.gz";
  try {
    writeFileSync(join(directory, name), fixture.data);
    writeFileSync(join(directory, `${name}.sig`), signer.signature);
    const input = { directory, version: "0.13.0", tag: "v0.13.0", trust };
    const manifest = createMacManifest(input);
    assert.equal(
      manifest.platforms["darwin-aarch64"].signature,
      signer.signature,
    );
    assert.equal(
      manifest.platforms["darwin-aarch64"].url,
      `https://github.com/KateNorth7/desktop_buddy/releases/download/v0.13.0/${name}`,
    );
    assert.equal(manifest.platforms["windows-x86_64"], undefined);
    const intelName = name.replace("aarch64", "x86_64");
    writeFileSync(join(directory, intelName), fixture.data);
    writeFileSync(join(directory, `${intelName}.sig`), signer.signature);
    assert.throws(() => createMacManifest(input), /Intel macOS/);
    rmSync(join(directory, intelName));
    rmSync(join(directory, `${intelName}.sig`));
    assert.throws(
      () => createMacManifest({ ...input, tag: "v0.12.0" }),
      /tag/i,
    );
    writeFileSync(join(directory, name), "tampered");
    assert.throws(() => createMacManifest(input), /verification/);
    writeFileSync(join(directory, name), fixture.data);
    assert.throws(
      () =>
        createMacManifest({
          ...input,
          trust: {
            epoch: 1,
            signers: [{ id: "other", pubkey: fixture.signers[1].pubkey }],
          },
        }),
      /not trusted/,
    );
    const otherName = "Desktop.Buddy_0.14.0_aarch64.app.tar.gz";
    writeFileSync(join(directory, otherName), fixture.data);
    writeFileSync(join(directory, `${otherName}.sig`), signer.signature);
    assert.throws(
      () => createMacManifest({ ...input, version: "0.14.0", tag: "v0.14.0" }),
      /version/,
    );
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
