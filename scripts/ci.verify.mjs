import { test } from "node:test";
import assert from "node:assert/strict";
import {
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  classify,
  normalizeVersion,
  fingerprint,
  inputKeys,
  POLICY,
} from "./ci-inputs.mjs";
import {
  readProof,
  rustProof,
  distDigest,
  fullProof,
  validReport,
  validLintReport,
  rustLintProof,
} from "./ci-proof.mjs";
import { validatePrivacyCopy } from "./check-privacy-source.mjs";
import { watch } from "./watch-ci.mjs";
import { prepare, validateNotes } from "./prepare-release.mjs";

const key = "a".repeat(64),
  now = Date.now();
const rust = {
  status: "passed",
  tests: [
    { name: "works", fullName: "suite::works", status: "passed" },
    { name: "live", fullName: "suite::live", status: "skipped" },
  ],
  counts: { passed: 1, failed: 0, skipped: 1 },
};
function temporary(fn) {
  const root = mkdtempSync(join(tmpdir(), "buddy-ci-"));
  try {
    return fn(root);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
function write(root, path, value) {
  const location = join(root, path);
  mkdirSync(join(location, ".."), { recursive: true });
  writeFileSync(
    location,
    typeof value === "string" || Buffer.isBuffer(value)
      ? value
      : JSON.stringify(value),
  );
  return location;
}
function proof(extra = {}) {
  return {
    policy: POLICY,
    key,
    kind: "rust",
    status: "passed",
    commit: "b".repeat(40),
    run: "https://github.com/ArtmarketVM/desktop_buddy/actions/runs/123",
    recordedAt: new Date(now).toISOString(),
    suite: rust,
    ...extra,
  };
}
function lintFixture(root, directory = "full/rust-lint") {
  const raw =
    JSON.stringify({ reason: "build-finished", success: true }) + "\n";
  const digest = createHash("sha256").update(raw).digest("hex");
  for (const profile of ["debug", "release"])
    write(root, `${directory}/${profile}.jsonl`, raw);
  return write(
    root,
    `${directory}/receipt.json`,
    proof({
      kind: "rust-lint",
      digests: { debug: digest, release: digest },
    }),
  );
}
function repository(root) {
  execFileSync("git", ["init", "-q"], { cwd: root });
  write(root, "src-tauri/src/lib.rs", "fn actual() {}\n");
  write(root, "src/data/rolePresets.json", "{}");
  write(root, "src/style.css", "body { color: black; }");
  write(root, "package.json", { version: "1.0.0", dependencies: { x: "1" } });
  write(root, "README.md", "original");
  write(root, "docs/privacy.html", "policy text");
  execFileSync("git", ["add", "."], { cwd: root });
}

test("documentation changes do not schedule native compilation or visual work", () => {
  assert.deepEqual(
    classify([
      "README.md",
      "AGENTS.md",
      "docs/SESSION_HANDOFF.md",
      "docs/privacy.html",
    ]),
    { checks: false, visual: false, rust: false },
  );
  assert.equal(
    classify(["docs/TEN_PHASES_ACCEPTANCE_CHECKLIST.md"]).checks,
    true,
  );
});
test("frontend, backend, visual and unknown changes select conservative gates", () => {
  assert.equal(classify(["src-tauri/embedded.md"]).checks, true);
  assert.equal(classify(["docs/unknown.html"]).checks, true);
  assert.deepEqual(classify(["src/style.css"]), {
    checks: true,
    visual: true,
    rust: false,
  });
  assert.deepEqual(classify(["src-tauri/src/lib.rs"]), {
    checks: true,
    visual: false,
    rust: true,
  });
  assert.equal(classify(["server/supabase/public.json"]).rust, true);
  assert.equal(
    classify(["server/supabase/functions/buddy-ai/handler.ts"]).checks,
    true,
  );
  assert.deepEqual(classify(["visual/screens.spec.ts"]), {
    checks: false,
    visual: true,
    rust: false,
  });
  assert.equal(classify(["unknown.build.config"]).checks, true);
  assert.equal(classify([".github/workflows/release.yml"]).visual, true);
});
test("only the application's version fields are normalized", () => {
  const a = Buffer.from('{"version":"1.0.0","dependencies":{"x":"1"}}');
  const b = Buffer.from('{"version":"1.0.1","dependencies":{"x":"1"}}');
  assert.deepEqual(
    normalizeVersion("package.json", a),
    normalizeVersion("package.json", b),
  );
  assert.notDeepEqual(
    normalizeVersion("package.json", a),
    normalizeVersion(
      "package.json",
      Buffer.from('{"version":"1.0.1","dependencies":{"x":"2"}}'),
    ),
  );
  const lock =
    '[[package]]\nname = "desktop-buddy"\nversion = "1.0.0"\n\n[[package]]\nname = "dependency"\nversion = "1.0.0"\n';
  assert.match(
    normalizeVersion("src-tauri/Cargo.lock", Buffer.from(lock)).toString(),
    /name = "dependency"\nversion = "1.0.0"/,
  );
  assert.deepEqual(
    normalizeVersion("src-tauri/native/helper", Buffer.from([255, 254])),
    Buffer.from([255, 254]),
  );
  assert.notDeepEqual(
    normalizeVersion("src-tauri/native/helper", Buffer.from([255])),
    normalizeVersion("src-tauri/native/helper", Buffer.from([254])),
  );
});
test("CSS and documentation preserve Rust keys while embedded data and backend changes invalidate them", () =>
  temporary((root) => {
    repository(root);
    const before = fingerprint(root, "rust"),
      full = fingerprint(root, "full");
    write(root, "docs/privacy.html", "corrected policy text");
    assert.equal(fingerprint(root, "full"), full);
    write(root, "src/style.css", "body { color: red; }");
    write(root, "README.md", "changed");
    assert.equal(fingerprint(root, "rust"), before);
    assert.notEqual(fingerprint(root, "full"), full);
    write(root, "src/data/rolePresets.json", '{"changed":true}');
    assert.notEqual(fingerprint(root, "rust"), before);
  }));
test("version bumps preserve Rust evidence but invalidate full frontend artifacts", () =>
  temporary((root) => {
    repository(root);
    const before = fingerprint(root, "rust"),
      full = fingerprint(root, "full");
    write(root, "package.json", { version: "1.0.1", dependencies: { x: "1" } });
    assert.equal(fingerprint(root, "rust"), before);
    assert.notEqual(fingerprint(root, "full"), full);
  }));
test("runner, toolchain, architecture and weekly expiry are part of proof keys", () =>
  temporary((root) => {
    repository(root);
    const host = { os: "win32", arch: "x64", rust: "1", image: "one" };
    const original = inputKeys(root, host, 0);
    for (const value of [
      { ...host, rust: "2" },
      { ...host, arch: "arm64" },
      { ...host, image: "two" },
      { ...host, os: "darwin" },
    ])
      assert.notEqual(inputKeys(root, value, 0).rust_key, original.rust_key);
    assert.notEqual(
      inputKeys(root, host, 7 * 86400000).rust_key,
      original.rust_key,
    );
  }));
test("failed, stale, malformed and wrong-input evidence is never reusable", () =>
  temporary((root) => {
    const path = write(root, "receipt.json", proof());
    assert.ok(readProof(path, key, "rust", now));
    for (const extra of [
      { key: "c".repeat(64) },
      { status: "failed" },
      { kind: "full" },
      { policy: "obsolete" },
      { recordedAt: new Date(now - 8 * 86400000).toISOString() },
      { recordedAt: "invalid" },
      { commit: "unknown" },
      { run: "untrusted" },
    ]) {
      writeFileSync(path, JSON.stringify(proof(extra)));
      assert.equal(readProof(path, key, "rust", now), null);
    }
    writeFileSync(path, "{malformed");
    assert.equal(readProof(path, key, "rust", now), null);
  }));
test("ignored Rust tests remain ignored and failures or mismatched counts prevent reuse", () =>
  temporary((root) => {
    const path = write(root, "receipt.json", proof());
    assert.equal(rustProof(path, key).suite.counts.skipped, 1);
    for (const suite of [
      { ...rust, counts: { passed: 2, failed: 0, skipped: 0 } },
      { ...rust, tests: [] },
      { ...rust, status: "failed" },
    ]) {
      writeFileSync(path, JSON.stringify(proof({ suite })));
      assert.equal(rustProof(path, key), null);
    }
  }));
test("full proofs require all acceptance suites and exact report and bundle bytes", () =>
  temporary((root) => {
    const dist = join(root, "dist");
    write(root, "dist/index.html", "<html>validated</html>");
    write(root, "dist/assets/app.js", "validated");
    const suites = Object.fromEntries(
      [
        "tooling",
        "versions",
        "frontend",
        "release",
        "typescript",
        "bundle",
      ].map((name) => [
        name,
        { status: "passed", counts: { passed: 1, failed: 0, tests: 1 } },
      ]),
    );
    suites.rust = rust;
    const report = { exitCode: 0, suites };
    assert.equal(validReport(report), true);
    assert.equal(validReport({ exitCode: 0, suites: { rust } }), false);
    const raw = JSON.stringify(report);
    write(root, "full/acceptance/report.json", raw);
    const path = write(
      root,
      "full/receipt.json",
      proof({
        kind: "full",
        rustLintKey: key,
        distDigest: distDigest(dist),
        reportDigest: createHash("sha256").update(raw).digest("hex"),
      }),
    );
    assert.equal(fullProof(path, key, dist), null);
    lintFixture(root);
    assert.ok(fullProof(path, key, dist));
    write(root, "dist/assets/app.js", "tampered");
    assert.equal(fullProof(path, key, dist), null);
    write(root, "dist/assets/app.js", "validated");
    write(
      root,
      "full/acceptance/report.json",
      JSON.stringify({ exitCode: 0, suites: { rust } }),
    );
    assert.equal(fullProof(path, key, dist), null);
  }));
test("Rust lint reuse requires both successful zero-warning profiles and exact report bytes", () =>
  temporary((root) => {
    const path = lintFixture(root);
    assert.ok(rustLintProof(path, key));
    assert.equal(rustLintProof(path, "different-inputs"), null);
    write(
      root,
      "full/rust-lint/release.jsonl",
      '{"reason":"build-finished","success":false}\n',
    );
    assert.equal(rustLintProof(path, key), null);
    for (const raw of [
      "",
      "not JSON",
      '{"reason":"build-finished","success":false}',
      '{"reason":"compiler-message","message":{"level":"warning"}}\n{"reason":"build-finished","success":true}',
      '{"reason":"compiler-message","message":{"level":"error"}}\n{"reason":"build-finished","success":true}',
    ])
      assert.equal(validLintReport(raw), false);
    assert.equal(
      validLintReport('{"reason":"build-finished","success":true}'),
      true,
    );
  }));
test("privacy corrections are checked before native compilation and stale claims fail fast", () => {
  const source =
    "pub(crate) const SAMPLE_INTERVAL: Duration = Duration::from_secs(20);";
  const policy =
    "at most one capture attempt per 20 seconds. encrypted temporary review evidence, deleted after the corresponding daily summary is successfully saved.";
  assert.doesNotThrow(() => validatePrivacyCopy(source, policy));
  assert.throws(
    () =>
      validatePrivacyCopy(source, policy.replace("20 seconds", "120 seconds")),
    /cadence/,
  );
  assert.throws(
    () =>
      validatePrivacyCopy(
        source,
        policy.replace(
          "encrypted temporary review evidence",
          "Images are temporary and are not saved",
        ),
      ),
    /encrypted/,
  );
  assert.throws(
    () => validatePrivacyCopy("unrecognized interval", policy),
    /cadence/,
  );
});
test("monitor emits stage changes once and retries bounded network errors without calling CI failed", async () => {
  const states = [
    { status: "in_progress", jobs: [] },
    { status: "in_progress", jobs: [] },
    { status: "completed", conclusion: "success", jobs: [] },
  ];
  const emitted = [],
    waits = [];
  assert.equal(
    await watch({
      read: () => states.shift(),
      emit: (v) => emitted.push(v),
      wait: async (ms) => waits.push(ms),
    }),
    0,
  );
  assert.equal(emitted.length, 2);
  assert.deepEqual(waits, [60000, 60000]);
  await assert.rejects(
    watch({
      read: () => {
        throw new Error("offline");
      },
      emit: () => {},
      wait: async () => {},
    }),
    /outcome is unknown/,
  );
});
test("release preparation validates complete notes and updates five manifests before CI", () =>
  temporary((root) => {
    for (const name of ["package.json", "src-tauri/tauri.conf.json"])
      write(root, name, { version: "1.0.0" });
    write(root, "package-lock.json", {
      version: "1.0.0",
      packages: { "": { version: "1.0.0" } },
    });
    write(
      root,
      "src-tauri/Cargo.toml",
      '[package]\nname = "desktop-buddy"\nversion = "1.0.0"\n',
    );
    write(
      root,
      "src-tauri/Cargo.lock",
      '[[package]]\nname = "desktop-buddy"\nversion = "1.0.0"\n',
    );
    mkdirSync(join(root, "docs"));
    assert.throws(() => validateNotes("# sumi 1.0.1\nTODO", "1.0.1"));
    assert.throws(() => prepare(root, "1.0.0", "notes"));
    const notes =
      "# sumi 1.0.1\n\n- Correct a specific user-visible issue in this release.\n";
    prepare(root, "1.0.1", notes);
    for (const path of [
      "package.json",
      "package-lock.json",
      "src-tauri/tauri.conf.json",
    ])
      assert.equal(JSON.parse(readFileSync(join(root, path))).version, "1.0.1");
    for (const path of ["src-tauri/Cargo.toml", "src-tauri/Cargo.lock"])
      assert.match(readFileSync(join(root, path), "utf8"), /version = "1.0.1"/);
    assert.equal(
      readFileSync(join(root, "docs/RELEASE_1.0.1.md"), "utf8"),
      notes,
    );
  }));
test("workflow policy retains both signed platforms, strict visual gating and main-only evidence writes", () => {
  const read = (path) =>
    readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
  const release = read(".github/workflows/release.yml"),
    platforms = read(".github/workflows/platform-checks.yml"),
    visual = read(".github/workflows/visual-checks.yml");
  assert.match(release, /needs: \[platforms, visual\]/);
  assert.match(release, /node scripts\/verify-release.mjs/);
  assert.match(release, /gh release create.*--draft/);
  assert.match(platforms, /aarch64-apple-darwin/);
  assert.match(platforms, /runner: windows-2022/);
  assert.doesNotMatch(platforms, /windows-latest/);
  assert.match(platforms, /x86_64-pc-windows-msvc/);
  assert.doesNotMatch(platforms, /x86_64-apple-darwin|universal-apple/);
  assert.match(
    platforms,
    /github.ref == 'refs\/heads\/main' && github.event_name != 'pull_request'/,
  );
  assert.match(platforms, /npm run test:acceptance/);
  assert.match(platforms, /shared-key: buddy-.*bundle-/);
  assert.match(platforms, /cargo fmt.*--check/);
  assert.match(platforms, /group: validation-\$\{\{ github.sha \}\}/);
  assert.match(visual, /npm run test:visual -- --reporter=line,json,html/);
  assert.match(visual, /inputs.candidates/);
  assert.doesNotMatch(visual, /--update-snapshots=all/);
  assert.doesNotMatch(release + platforms + visual, /pull_request_target/);
});
test("lint gates block and their warning budgets cannot be bypassed", () => {
  const read = (path) =>
    readFileSync(new URL(`../${path}`, import.meta.url), "utf8").replaceAll(
      "\r\n",
      "\n",
    );
  const ci = read(".github/workflows/ci.yml"),
    platforms = read(".github/workflows/platform-checks.yml");
  assert.doesNotMatch(ci + platforms, /continue-on-error/);
  assert.match(platforms, /run: npm run lint\n/);
  assert.match(platforms, /run: npm run lint:unused\n/);
  assert.match(platforms, /run: npm run lint:rust\n/);
  assert.match(
    platforms,
    /cargo clippy --locked --release --manifest-path src-tauri\/Cargo.toml --all-targets --message-format=json -- -D warnings/,
  );
  assert.match(ci, /needs: \[plan, platforms, visual, rust-lint\]/);
  assert.match(ci, /rust-lint:\n    needs: \[plan, platforms\]\n/);
  assert.doesNotMatch(ci, /runner: macos-15/);
  assert.match(
    platforms,
    /runs-on: \$\{\{ \(matrix\.name == 'macos-apple-silicon' && github\.event_name != 'pull_request' && !inputs\.hosted_macos\) && fromJSON\('\["self-hosted","macOS","ARM64","sumi"\]'\) \|\| matrix\.runner \}\}/,
  );
  const release = read(".github/workflows/release.yml");
  assert.match(
    release,
    /hosted_macos:\n\s+description: [^\n]+\n\s+type: boolean\n\s+default: false/,
  );
  assert.match(release, /hosted_macos: \$\{\{ inputs.hosted_macos \}\}/);
  assert.match(platforms, /hosted_macos:\n\s+type: boolean\n\s+default: false/);
  assert.match(platforms, /run: test "\$\(uname -m\)" = arm64/);
  assert.match(
    platforms,
    /group: validation-.*inputs.hosted_macos && matrix.name == 'macos-apple-silicon' && '-hosted'/,
  );
  assert.match(platforms, /components: rustfmt, clippy/);
  assert.match(platforms, /toolchain: 1.99.1/);
  assert.match(platforms, /ci-proof.mjs lint-record/);
  assert.ok(
    platforms.indexOf("Provide a placeholder frontend") <
      platforms.indexOf("run: npm run lint:rust"),
  );
  assert.doesNotMatch(ci, /^concurrency:/m);
  assert.match(
    ci,
    /platforms:\n    needs: plan\n    if: needs.plan.outputs.checks == 'true'\n    concurrency:/,
  );
  assert.match(ci, /test "\$RUST_LINT" = success/);
  const budget = JSON.parse(read("scripts/lint-budget.json"));
  assert.ok(Number.isInteger(budget.eslintWarnings));
  assert.ok(Number.isInteger(budget.clippyWarnings));
  assert.ok(Number.isInteger(budget.unusedCode));
  assert.match(read("scripts/lint-frontend.mjs"), /counts\.warnings > budget/);
  assert.match(read("scripts/lint-rust.mjs"), /summary\.warnings > budget/);
  assert.match(read("scripts/lint-unused.mjs"), /unique\.length > budget/);
});
