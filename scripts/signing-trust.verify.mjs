import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { publicKey, validateTrust, verifyArtifact } from "./signing-trust.mjs";
import { addSigner } from "./add-trusted-signer.mjs";
import { prepareChannels } from "./create-update-manifest.mjs";

const fixture = JSON.parse(
  readFileSync(
    new URL("../src-tauri/test-data/signing.json", import.meta.url),
    "utf8",
  ),
);
const [a, b, unknown] = fixture.signers;
const signer = ({ id, pubkey }) => ({ id, pubkey });
const oldTrust = { epoch: 1, signers: [signer(a)] };
const team = { epoch: 2, signers: [signer(a), signer(b)] };
const bytes = Buffer.from(fixture.data);
const manifest = (key, version) => ({
  version,
  platforms: {
    "windows-x86_64": {
      signature: key.versions[version],
      url: `https://github.com/KateNorth7/desktop_buddy/releases/download/v${version}/Desktop.Buddy_${version}_x64-setup.exe`,
    },
  },
});

test("two independent trusted developers can sign; unknown, tampered and wrong-version artifacts fail", () => {
  for (const key of [a, b])
    assert.equal(
      verifyArtifact(bytes, key.signature, team, fixture.version),
      key.id,
    );
  assert.throws(
    () => verifyArtifact(bytes, unknown.signature, team, fixture.version),
    /not trusted/,
  );
  assert.throws(
    () =>
      verifyArtifact(
        Buffer.from("tampered"),
        a.signature,
        team,
        fixture.version,
      ),
    /verification failed/,
  );
  assert.throws(
    () => verifyArtifact(bytes, a.signature, team, "0.13.1"),
    /version/,
  );
  const comment = Buffer.from(a.signature, "base64")
    .toString()
    .replace("version:0.13.0", "version:0.13.1");
  assert.throws(
    () =>
      verifyArtifact(
        bytes,
        Buffer.from(comment).toString("base64"),
        team,
        "0.13.1",
      ),
    /verification failed/,
  );
});

test("enrollment accepts only a public key and rejects duplicate names or keys", () => {
  assert.equal(
    publicKey(Buffer.from(a.pubkey, "base64").toString()).id,
    publicKey(a.pubkey).id,
  );
  const next = addSigner(oldTrust, b.id, b.pubkey);
  assert.equal(next.epoch, 2);
  assert.equal(next.signers.length, 2);
  assert.throws(() => addSigner(oldTrust, a.id, b.pubkey), /unique/);
  assert.throws(() => addSigner(oldTrust, b.id, a.pubkey), /Duplicate/);
  assert.throws(
    () => addSigner(oldTrust, b.id, "private key material"),
    /public/,
  );
  assert.throws(() => validateTrust(team, unknown.pubkey), /default/);
});

test("older installations receive their bridge even when they skip releases", () => {
  const initial = prepareChannels({
    manifest: manifest(a, "0.13.0"),
    bytes,
    trust: oldTrust,
    history: {},
    compatibility: {},
  });
  const transition = prepareChannels({
    manifest: manifest(a, "0.13.1"),
    bytes,
    trust: team,
    history: { 1: oldTrust },
    compatibility: initial.archive,
  });
  const later = prepareChannels({
    manifest: manifest(b, "0.14.0"),
    bytes,
    trust: team,
    history: { 1: oldTrust },
    compatibility: transition.archive,
  });
  assert.equal(later.files["latest.json"].version, "0.13.0");
  assert.equal(later.files["updates-epoch-1.json"].version, "0.13.1");
  assert.equal(later.files["updates-epoch-2.json"].version, "0.14.0");
});

test("a third signer needs an epoch-2 bridge signed by an existing developer", () => {
  const initial = prepareChannels({
    manifest: manifest(a, "0.13.0"), bytes, trust: oldTrust,
    history: {}, compatibility: {},
  });
  const second = prepareChannels({
    manifest: manifest(a, "0.13.1"), bytes, trust: team,
    history: { 1: oldTrust }, compatibility: initial.archive,
  });
  const third = addSigner(team, unknown.id, unknown.pubkey);
  const options = { bytes, trust: third, history: { 1: oldTrust, 2: team }, compatibility: second.archive };
  assert.throws(() => prepareChannels({ ...options, manifest: manifest(unknown, "0.14.0") }), /not trusted/);
  const transition = prepareChannels({ ...options, manifest: manifest(a, "0.14.0") });
  assert.equal(third.epoch, 3);
  assert.equal(transition.files["updates-epoch-1.json"].version, "0.13.1");
  assert.equal(transition.files["updates-epoch-2.json"].version, "0.14.0");
  assert.deepEqual(transition.files["updates-epoch-3.json"], transition.files["updates-epoch-2.json"]);
  for (const key of [a, b, unknown])
    assert.equal(verifyArtifact(bytes, key.versions["0.14.0"], third, "0.14.0"), key.id);
});

test("a new developer cannot authorize their own transition or skip a required bridge", () => {
  const initial = prepareChannels({
    manifest: manifest(a, "0.13.0"),
    bytes,
    trust: oldTrust,
    history: {},
    compatibility: {},
  });
  assert.throws(
    () =>
      prepareChannels({
        manifest: manifest(b, "0.13.1"),
        bytes,
        trust: team,
        history: { 1: oldTrust },
        compatibility: initial.archive,
      }),
    /not trusted/,
  );
  assert.throws(
    () =>
      prepareChannels({
        manifest: manifest(a, "0.13.1"),
        bytes,
        trust: team,
        history: {},
        compatibility: initial.archive,
      }),
    /missing/,
  );
  assert.throws(
    () =>
      prepareChannels({
        manifest: manifest(a, "0.13.1"),
        bytes,
        trust: team,
        history: { 1: oldTrust },
        compatibility: {},
      }),
    /initial legacy bridge/,
  );
});

test("archived bridge versions cannot be overwritten or exceed the current release", () => {
  const initial = prepareChannels({
    manifest: manifest(a, "0.13.0"),
    bytes,
    trust: oldTrust,
    history: {},
    compatibility: {},
  });
  assert.throws(
    () =>
      prepareChannels({
        manifest: manifest(b, "0.13.0"),
        bytes,
        trust: { ...team, epoch: 1 },
        history: {},
        compatibility: initial.archive,
      }),
    /different signature/,
  );
  const future = { "latest.json": manifest(a, "0.14.0") };
  assert.throws(
    () =>
      prepareChannels({
        manifest: manifest(a, "0.13.0"),
        bytes,
        trust: oldTrust,
        history: {},
        compatibility: future,
      }),
    /newer/,
  );
});
