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
import { assertNewerRelease, verifyRelease } from "./verify-release.mjs";
import { createMacManifest } from "./create-macos-update-manifest.mjs";

test("release publication refuses equal, older and malformed version tags", () => {
  assert.doesNotThrow(() => assertNewerRelease("v0.23.2", "v0.23.1"));
  assert.doesNotThrow(() => assertNewerRelease("v0.24.0", "v0.23.9"));
  assert.doesNotThrow(() => assertNewerRelease("v1.0.0", "v0.99.99"));
  assert.doesNotThrow(() => assertNewerRelease("v0.39.0", "0.38.0"));
  assert.doesNotThrow(() => assertNewerRelease("v0.50.0", ""));
  assert.doesNotThrow(() => assertNewerRelease("v0.50.0", " \n"));
  for (const [candidate, latest] of [
    ["v0.23.2", "v0.23.2"],
    ["v0.23.2", "v0.23.3"],
    ["v0.23.2", "v0.24.0"],
    ["v0.23.2", "v1.0.0"],
    ["invalid", "v0.23.1"],
    ["v0.39.0", "0.39.0"],
    ["v0.39.0", "0.40.0"],
    ["0.39.0", "0.38.0"],
    ["v0.39.0", "invalid"],
  ])
    assert.throws(() => assertNewerRelease(candidate, latest));
});

function fixture() {
  const data = JSON.parse(
    readFileSync(
      new URL("../src-tauri/test-data/signing.json", import.meta.url),
    ),
  );
  const signer = data.signers[0];
  const directory = mkdtempSync(join(tmpdir(), "buddy-complete-release-"));
  mkdirSync(join(directory, "src-tauri"));
  mkdirSync(join(directory, "updates"));
  const trust = {
    epoch: 1,
    signers: [{ id: "primary", pubkey: signer.pubkey }],
  };
  const write = (name, value) =>
    writeFileSync(join(directory, name), JSON.stringify(value));
  write("package.json", { version: "0.13.0" });
  write("src-tauri/trusted-signers.json", trust);
  write("src-tauri/trusted-signers.macos.json", trust);
  write("src-tauri/tauri.conf.json", {
    plugins: { updater: { pubkey: signer.pubkey } },
  });
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
  write("updates/compatibility.json", { "latest.json": manifest });
  write("latest.json", manifest);
  write("updates-epoch-1.json", manifest);
  writeFileSync(join(directory, name), data.data);
  writeFileSync(join(directory, name + ".sig"), signer.signature);
  for (const arch of ["aarch64"]) {
    const base = `Desktop.Buddy_0.13.0_${arch}`;
    writeFileSync(join(directory, base + ".app.tar.gz"), data.data);
    writeFileSync(join(directory, base + ".app.tar.gz.sig"), signer.signature);
    writeFileSync(join(directory, base + ".dmg"), "installer fixture");
    writeFileSync(join(directory, base + ".zip"), "installer fixture");
  }
  const macInput = { directory, version: "0.13.0", tag: "v0.13.0", trust };
  write("updates-macos-epoch-1.json", createMacManifest(macInput));
  return { directory, write, macInput };
}

test("complete release accepts verified matching Windows and macOS assets", () => {
  const { directory } = fixture();
  try {
    assert.equal(verifyRelease(directory, "v0.13.0", directory), "0.13.0");
    assert.throws(() => verifyRelease(directory, "v0.14.0", directory), /tag/);
    writeFileSync(
      join(directory, "package.json"),
      JSON.stringify({ version: "0.14.0" }),
    );
    assert.throws(
      () => verifyRelease(directory, "v0.14.0", directory),
      /same version/,
    );
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("complete release rejects a missing Apple Silicon archive and empty installers", () => {
  const { directory } = fixture();
  try {
    const dmg = join(directory, "Desktop.Buddy_0.13.0_aarch64.dmg");
    writeFileSync(dmg, "");
    assert.throws(
      () => verifyRelease(directory, "v0.13.0", directory),
      /empty/,
    );
    writeFileSync(dmg, "installer fixture");
    rmSync(join(directory, "Desktop.Buddy_0.13.0_aarch64.app.tar.gz"));
    assert.throws(
      () => verifyRelease(directory, "v0.13.0", directory),
      /Missing macOS updater archive/,
    );
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("complete release rejects altered archives, channels and compatibility bridges", () => {
  const { directory, write, macInput } = fixture();
  try {
    const manifest = createMacManifest(macInput);
    manifest.platforms["darwin-aarch64"].url =
      "https://example.com/update.tar.gz";
    write("updates-macos-epoch-1.json", manifest);
    assert.throws(
      () => verifyRelease(directory, "v0.13.0", directory),
      /channel/,
    );
    write("updates-macos-epoch-1.json", createMacManifest(macInput));
    write("latest.json", { version: "0.12.0" });
    assert.throws(
      () => verifyRelease(directory, "v0.13.0", directory),
      /compatibility/,
    );
    write(
      "latest.json",
      JSON.parse(readFileSync(join(directory, "updates/compatibility.json")))[
        "latest.json"
      ],
    );
    writeFileSync(
      join(directory, "Desktop.Buddy_0.13.0_aarch64.app.tar.gz"),
      "tampered",
    );
    assert.throws(
      () => verifyRelease(directory, "v0.13.0", directory),
      /verification/,
    );
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
