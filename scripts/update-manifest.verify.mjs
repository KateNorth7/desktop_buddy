import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createManifest } from "./create-update-manifest.mjs";

test("publishes a signed, version-pinned Windows installer URL", () => {
  const directory = mkdtempSync(join(tmpdir(), "buddy-release-"));
  const name = "Desktop Buddy_0.10.0_x64-setup.exe";
  const signature = Buffer.from("untrusted comment: fixture\nfixture\ntrusted comment: fixture\nfixture\n").toString("base64");
  try {
    writeFileSync(join(directory, name), "fixture");
    writeFileSync(join(directory, `${name}.sig`), signature);
    const manifest = createManifest({directory, version:"0.10.0", tag:"v0.10.0"});
    assert.equal(manifest.platforms["windows-x86_64"].signature, signature);
    assert.equal(manifest.platforms["windows-x86_64"].url, "https://github.com/KateNorth7/desktop_buddy/releases/download/v0.10.0/Desktop.Buddy_0.10.0_x64-setup.exe");
    assert.throws(() => createManifest({directory, version:"0.10.0", tag:"v0.9.0"}), /tag/);
    writeFileSync(join(directory, `${name}.sig`), "");
    assert.throws(() => createManifest({directory, version:"0.10.0", tag:"v0.10.0"}), /signature/);
    writeFileSync(join(directory, "Other_0.10.0_x64-setup.exe"), "fixture");
    assert.throws(() => createManifest({directory, version:"0.10.0", tag:"v0.10.0"}), /exactly one/);
  } finally { rmSync(directory, {recursive:true, force:true}); }
});
