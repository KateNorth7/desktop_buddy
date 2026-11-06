import { test } from "node:test";
import assert from "node:assert/strict";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { verifyRetainedWindows } from "./verify-retained-windows.mjs";

test("retained Windows channels reject changed bridges, URLs and installer bytes", () => {
  const fixture = JSON.parse(
    readFileSync(
      new URL("../src-tauri/test-data/signing.json", import.meta.url),
    ),
  );
  const directory = mkdtempSync(join(tmpdir(), "buddy-retained-test-"));
  const signer = fixture.signers[0];
  const name = "Desktop.Buddy_0.13.0_x64-setup.exe";
  const manifest = {
    version: "0.13.0",
    platforms: {
      "windows-x86_64": {
        signature: signer.signature,
        url: `https://github.com/KateNorth7/desktop_buddy/releases/download/v0.13.0/${name}`,
      },
    },
  };
  const write = (name, value) =>
    writeFileSync(join(directory, name), JSON.stringify(value));
  try {
    mkdirSync(join(directory, "src-tauri"));
    mkdirSync(join(directory, "updates"));
    write("src-tauri/trusted-signers.json", {
      epoch: 1,
      signers: [{ id: "primary", pubkey: signer.pubkey }],
    });
    write("src-tauri/tauri.conf.json", {
      plugins: { updater: { pubkey: signer.pubkey } },
    });
    write("updates/compatibility.json", { "latest.json": manifest });
    write("latest.json", manifest);
    write("updates-epoch-1.json", manifest);
    writeFileSync(join(directory, name), fixture.data);
    writeFileSync(join(directory, `${name}.sig`), signer.signature);
    assert.equal(verifyRetainedWindows(directory, directory), "0.13.0");
    write("latest.json", { ...manifest, version: "0.12.0" });
    assert.throws(
      () => verifyRetainedWindows(directory, directory),
      /compatibility/,
    );
    write("latest.json", manifest);
    const altered = structuredClone(manifest);
    altered.platforms["windows-x86_64"].url = "https://example.com/update.exe";
    write("updates-epoch-1.json", altered);
    assert.throws(() => verifyRetainedWindows(directory, directory), /pinned/);
    write("updates-epoch-1.json", manifest);
    writeFileSync(join(directory, name), "altered installer");
    assert.throws(
      () => verifyRetainedWindows(directory, directory),
      /verification/,
    );
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
