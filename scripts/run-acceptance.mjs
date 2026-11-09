import { rustProof, writeProof } from "./ci-proof.mjs";
import { spawn, spawnSync } from "node:child_process";
import {
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { arch, platform, release } from "node:os";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { coverage } from "./acceptance-coverage.mjs";

export const projectRoot = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "..",
);
export function readChecklist(text, mapping = coverage) {
  const items = [...text.matchAll(/^- \[[ x]\] ([A-Z]+\d*-\d+) — (.+)$/gm)].map(
    (match) => ({ id: match[1], description: match[2] }),
  );
  if (!items.length)
    throw new Error("No acceptance IDs found in the checklist");
  const seen = new Set();
  for (const item of items) {
    if (seen.has(item.id))
      throw new Error(`Duplicate checklist ID: ${item.id}`);
    seen.add(item.id);
    const entry = mapping[item.id];
    if (!entry) throw new Error(`Unmapped checklist ID: ${item.id}`);
    if (!["contract", "partial", "manual"].includes(entry.scope))
      throw new Error(`Invalid scope: ${item.id}`);
    if (entry.scope !== "manual" && !entry.evidence.length)
      throw new Error(`Missing evidence: ${item.id}`);
    if (entry.scope !== "contract" && !entry.remaining)
      throw new Error(`Missing manual instructions: ${item.id}`);
  }
  for (const id of Object.keys(mapping))
    if (!seen.has(id)) throw new Error(`Unknown mapped ID: ${id}`);
  return items;
}

export function parseRust(text) {
  const results = [];
  for (const match of text.matchAll(
    /^test ([\w:]+)(?: - should panic)? \.\.\. (ok|FAILED|ignored)(?:,.*)?\r?$/gm,
  )) {
    results.push({
      name: match[1].split("::").at(-1),
      fullName: match[1],
      status: { ok: "passed", FAILED: "failed", ignored: "skipped" }[match[2]],
    });
  }
  return results;
}

export function rustSummary(text) {
  const counts = { passed: 0, failed: 0, skipped: 0 };
  for (const match of text.matchAll(
    /^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;/gm,
  )) {
    counts.passed += Number(match[1]);
    counts.failed += Number(match[2]);
    counts.skipped += Number(match[3]);
  }
  return counts;
}

export function parseFrontend(json, root = projectRoot) {
  return (json.testResults ?? []).flatMap((file) => {
    const fileName = relative(root, file.name).replaceAll("\\", "/");
    return (file.assertionResults ?? []).map((test) => ({
      name: `${fileName}::${test.title}`,
      fullName: `${fileName}::${test.fullName ?? test.title}`,
      status:
        test.status === "passed"
          ? "passed"
          : test.status === "failed"
            ? "failed"
            : "skipped",
    }));
  });
}

export function evaluate(item, entry, suites) {
  const observations = [];
  for (const selector of entry.evidence) {
    const colon = selector.indexOf(":");
    const kind = selector.slice(0, colon);
    const name = selector.slice(colon + 1);
    const suite = suites[kind === "suite" ? name : kind];
    if (!suite || suite.status === "blocked" || suite.status === "planned") {
      observations.push({
        selector,
        status: "blocked",
        reason: suite?.reason ?? "Suite did not run",
      });
    } else if (kind === "suite") {
      observations.push({
        selector,
        status: suite.status === "passed" ? "passed" : "failed",
        reason: suite.reason,
      });
    } else {
      const matches = (suite.tests ?? []).filter((test) => test.name === name);
      if (matches.length !== 1) {
        observations.push({
          selector,
          status: "failed",
          reason: `Expected one test result, found ${matches.length}; update the mapping or investigate incomplete output`,
        });
      } else {
        observations.push({
          selector,
          status:
            matches[0].status === "skipped" ? "blocked" : matches[0].status,
          test: matches[0].fullName,
          reason:
            matches[0].status === "skipped"
              ? "Ignored or skipped tests cannot establish a pass"
              : undefined,
        });
      }
    }
  }
  const status =
    entry.scope === "manual"
      ? "manual"
      : observations.some((result) => result.status === "failed")
        ? "failed"
        : observations.some((result) => result.status === "blocked")
          ? "blocked"
          : entry.scope === "partial"
            ? "partial-pass"
            : "automated-pass";
  return { ...item, ...entry, status, observations };
}

export function exitCode(suites, items) {
  if (
    Object.values(suites).some((suite) => suite.status === "failed") ||
    items.some((item) => item.status === "failed")
  )
    return 1;
  if (
    Object.values(suites).some((suite) => suite.status === "blocked") ||
    items.some((item) => item.status === "blocked")
  )
    return 2;
  return 0;
}

function redact(text) {
  let result = text;
  for (const [name, value] of Object.entries(process.env)) {
    if (
      /(?:KEY|TOKEN|PASSWORD|SECRET)/i.test(name) &&
      value &&
      value.length >= 6
    )
      result = result.split(value).join("[REDACTED]");
  }
  return result;
}

async function command(spec, output) {
  console.log(`[acceptance] Running ${spec.id}`);
  const started = Date.now();
  const log = createWriteStream(join(output, `${spec.id}.log`));
  let captured = "";
  let timedOut = false;
  const childEnv = { ...process.env, NO_COLOR: "1", CARGO_TERM_COLOR: "never" };
  // Normal suites use synthetic fixtures and local HTTP servers; ignored live tests stay ignored.
  for (const name of Object.keys(childEnv)) {
    if (
      /^(?:ANTHROPIC|OPENAI|TAURI_SIGNING)_|^BUDDY_LIVE_DIAGNOSTICS$/.test(
        name,
      )
    )
      delete childEnv[name];
  }
  return await new Promise((done) => {
    const child = spawn(spec.command, spec.args, {
      cwd: projectRoot,
      env: childEnv,
      windowsHide: true,
    });
    const timer = setTimeout(
      () => {
        timedOut = true;
        if (process.platform === "win32")
          spawnSync("taskkill", ["/pid", String(child.pid), "/t", "/f"], {
            windowsHide: true,
          });
        else child.kill("SIGTERM");
      },
      30 * 60 * 1000,
    );
    const collect = (chunk) => {
      const text = redact(chunk.toString());
      log.write(text);
      if (captured.length < 20 * 1024 * 1024) captured += text;
    };
    child.stdout.on("data", collect);
    child.stderr.on("data", collect);
    let spawnError;
    child.on("error", (error) => {
      spawnError = error;
    });
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      log.end(() => {
        const status =
          spawnError?.code === "ENOENT"
            ? "blocked"
            : code === 0 && !timedOut
              ? "passed"
              : "failed";
        const reason = spawnError
          ? redact(spawnError.message)
          : timedOut
            ? "Suite exceeded the 30-minute timeout"
            : code !== 0
              ? `Exit ${code}; see ${spec.id}.log${signal ? ` (${signal})` : ""}`
              : undefined;
        console.log(
          `[acceptance] ${spec.id}: ${status} (${Math.round((Date.now() - started) / 1000)}s)`,
        );
        done({
          id: spec.id,
          status,
          reason,
          durationMs: Date.now() - started,
          command: [spec.command, ...spec.args],
          log: `${spec.id}.log`,
          captured,
          tests: [],
        });
      });
    });
  });
}

const cell = (text) =>
  String(text ?? "")
    .replaceAll("|", "\\|")
    .replaceAll("\n", " ");
export function markdown(report) {
  const lines = [
    "# sumi automated acceptance report",
    "",
    `Version: ${report.version}. Source commit: ${report.commit}. Source tree: ${report.dirty ? "contains local changes" : "clean"}.`,
    `Platform: ${report.platform} ${report.architecture}; OS: ${report.osRelease}. Started: ${report.startedAt}.`,
    "",
    "This report verifies deterministic test contracts on this host. Mocked IPC and isolated HTTP servers are not live model, microphone, installed-app, or updater validation. Original acceptance boxes remain unchanged. The other platform still requires its own run and native build evidence.",
    "",
    `Mode: ${report.mode}. Exit code: ${report.exitCode}. Counts: ${Object.entries(
      report.counts,
    )
      .map(([key, value]) => `${key} ${value}`)
      .join(", ")}.`,
    "",
    "automated-pass means all mapped contract evidence passed. partial-pass means mapped evidence passed but the Remaining verification column is still required. manual and blocked are never acceptance passes. Exit 0 means automated gates passed, not complete product acceptance.",
    "",
    "## Suite results",
    "",
    "| Suite | Result | Tests | Evidence |",
    "| --- | --- | --- | --- |",
  ];
  for (const suite of Object.values(report.suites)) {
    const status = suite.reusedFrom ? `${suite.status} (reused)` : suite.status;
    const evidence = suite.reusedFrom
      ? `Reused from ${suite.reusedFrom.run}; ${suite.log}`
      : (suite.reason ?? suite.log ?? "Not executed");
    lines.push(
      `| ${suite.id} | ${status} | ${cell(suite.counts ? JSON.stringify(suite.counts) : "See log")} | ${cell(evidence)} |`,
    );
  }
  lines.push(
    "",
    "## Checklist evidence",
    "",
    "| ID | Result | Scope | Evidence | Remaining verification |",
    "| --- | --- | --- | --- | --- |",
  );
  for (const item of report.items) {
    const evidence = item.observations
      .map(
        (result) =>
          `${result.selector}: ${result.status}${result.reason ? ` (${result.reason})` : ""}`,
      )
      .join("; ");
    lines.push(
      `| ${item.id} | ${item.status} | ${item.scope} | ${cell(evidence || "No automated evidence")} | ${cell(item.remaining)} |`,
    );
  }
  return `${lines.join("\n")}\n`;
}

export async function main(args = process.argv.slice(2)) {
  let plan = false;
  let rustEvidence;
  let rustKey;
  let output = join(
    projectRoot,
    ".tools",
    "acceptance",
    new Date().toISOString().replaceAll(/[:.]/g, "-"),
  );
  for (let index = 0; index < args.length; index++) {
    if (args[index] === "--plan") plan = true;
    else if (args[index] === "--rust-evidence" && args[index + 1])
      rustEvidence = resolve(projectRoot, args[++index]);
    else if (args[index] === "--rust-key" && args[index + 1])
      rustKey = args[++index];
    else if (args[index] === "--output" && args[index + 1])
      output = resolve(projectRoot, args[++index]);
    else throw new Error(`Unknown or incomplete argument: ${args[index]}`);
  }
  if (Boolean(rustEvidence) !== Boolean(rustKey))
    throw new Error("Rust evidence requires both a path and an input key");
  if (
    rustEvidence &&
    (relative(projectRoot, rustEvidence).startsWith("..") ||
      !/^[a-f0-9]{64}$/.test(rustKey))
  )
    throw new Error("Invalid Rust evidence location or key");
  const outputRelative = relative(projectRoot, output);
  if (
    !outputRelative ||
    outputRelative.startsWith("..") ||
    isAbsolute(outputRelative)
  )
    throw new Error("Report directory must be inside the workspace");
  // Never reuse a run directory or load test evidence from an earlier run.
  if (existsSync(output))
    throw new Error("Report directory already exists; choose a new directory");
  const items = readChecklist(
    readFileSync(
      join(projectRoot, "docs/TEN_PHASES_ACCEPTANCE_CHECKLIST.md"),
      "utf8",
    ),
  );
  const pkg = JSON.parse(
    readFileSync(join(projectRoot, "package.json"), "utf8"),
  );
  if (
    pkg.scripts.test !== "vitest run" ||
    pkg.scripts.build !==
      "node scripts/check-version.mjs && tsc -b && vite build"
  ) {
    throw new Error(
      "Standard test/build commands changed; update the acceptance runner before replacing those gates",
    );
  }
  const releaseScript = pkg.scripts["test:release"];
  if (!/^node --test (?:scripts\/[\w.-]+\.mjs ?)+$/.test(releaseScript))
    throw new Error(
      "Unsupported test:release command; update the acceptance runner explicitly",
    );
  const localCargo = join(
    projectRoot,
    ".tools/cargo/bin",
    process.platform === "win32" ? "cargo.exe" : "cargo",
  );
  const cargo = existsSync(localCargo) ? localCargo : "cargo";
  if (existsSync(localCargo)) {
    process.env.CARGO_HOME = join(projectRoot, ".tools/cargo");
    process.env.RUSTUP_HOME = join(projectRoot, ".tools/rustup");
    process.env.PATH = `${dirname(cargo)}${process.platform === "win32" ? ";" : ":"}${process.env.PATH}`;
  }
  const specs = [
    {
      id: "tooling",
      command: process.execPath,
      args: ["--test", "--test-reporter=tap", "scripts/acceptance.verify.mjs"],
    },
    {
      id: "versions",
      command: process.execPath,
      args: ["scripts/check-version.mjs"],
    },
    {
      id: "frontend",
      command: process.execPath,
      args: [
        "node_modules/vitest/vitest.mjs",
        "run",
        "--reporter=json",
        `--outputFile=${join(output, "frontend-results.json")}`,
      ],
    },
    {
      id: "release",
      command: process.execPath,
      args: [
        "--test",
        "--test-reporter=tap",
        ...releaseScript.split(" ").slice(2),
      ],
    },
    {
      id: "typescript",
      command: process.execPath,
      args: ["node_modules/typescript/bin/tsc", "-b"],
    },
    {
      id: "bundle",
      command: process.execPath,
      args: ["node_modules/vite/bin/vite.js", "build"],
    },
    {
      id: "rust",
      command: cargo,
      args: [
        "test",
        "--locked",
        "--manifest-path",
        "src-tauri/Cargo.toml",
        "--",
        "--color",
        "never",
      ],
    },
  ];
  const git = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: projectRoot,
    encoding: "utf8",
    windowsHide: true,
  });
  const status = spawnSync("git", ["status", "--porcelain"], {
    cwd: projectRoot,
    encoding: "utf8",
    windowsHide: true,
  });
  const report = {
    version: pkg.version,
    commit: git.status === 0 ? git.stdout.trim() : "unavailable",
    dirty: status.status === 0 ? Boolean(status.stdout.trim()) : null,
    platform: platform(),
    architecture: arch(),
    osRelease: release(),
    startedAt: new Date().toISOString(),
    mode: plan ? "plan only" : "synthetic automated contracts",
    suites: {},
    items: [],
    counts: {},
    exitCode: 0,
  };
  mkdirSync(output, { recursive: true });
  for (const spec of specs) {
    if (plan) {
      report.suites[spec.id] = {
        id: spec.id,
        status: "planned",
        command: [spec.command, ...spec.args],
        reason: "Plan only; no test executed",
      };
      continue;
    }
    if (
      spec.id === "bundle" &&
      ["versions", "typescript"].some(
        (id) => report.suites[id].status !== "passed",
      )
    ) {
      report.suites.bundle = {
        id: "bundle",
        status: "blocked",
        reason:
          "Build stopped because version or TypeScript checks did not pass",
      };
      continue;
    }
    if (spec.id === "rust" && rustEvidence) {
      const proof = rustProof(rustEvidence, rustKey);
      if (proof) {
        report.suites.rust = {
          ...proof.suite,
          reusedFrom: {
            commit: proof.commit,
            run: proof.run,
            recordedAt: proof.recordedAt,
          },
        };
        report.suites.rust.log = "rust-reused.log";
        writeFileSync(
          join(output, "rust-reused.log"),
          `Rust tests were not executed in this run.\nMatching validated inputs reused from commit ${proof.commit}.\nSource evidence: ${proof.run}\nCounts: ${JSON.stringify(proof.suite.counts)}\n`,
        );
        writeFileSync(
          join(output, "suite-progress.json"),
          JSON.stringify(report.suites, null, 2),
        );
        console.log(
          `[acceptance] rust: reused verified matching inputs from ${proof.run}`,
        );
        continue;
      }
    }
    const suite = await command(spec, output);
    if (spec.id === "frontend") {
      try {
        suite.tests = parseFrontend(
          JSON.parse(
            readFileSync(join(output, "frontend-results.json"), "utf8"),
          ),
        );
      } catch {
        suite.status = "failed";
        suite.reason =
          "No valid frontend JSON results produced; see frontend.log";
      }
    } else if (spec.id === "rust") {
      suite.tests = parseRust(suite.captured);
    }
    if (["frontend", "rust"].includes(spec.id)) {
      suite.counts = { passed: 0, failed: 0, skipped: 0 };
      for (const test of suite.tests) suite.counts[test.status]++;
      if (
        spec.id === "rust" &&
        suite.status === "passed" &&
        JSON.stringify(suite.counts) !==
          JSON.stringify(rustSummary(suite.captured))
      ) {
        suite.status = "failed";
        suite.reason =
          "Parsed Rust evidence does not match Cargo summary counts";
      }
      if (suite.status === "passed" && !suite.tests.length) {
        suite.status = "failed";
        suite.reason =
          "Suite exited successfully but produced no recognized test evidence";
      }
    }
    if (["tooling", "release"].includes(spec.id)) {
      suite.counts = Object.fromEntries(
        [
          ...suite.captured.matchAll(
            /^# (tests|pass|fail|skipped) (\d+)\r?$/gm,
          ),
        ].map((match) => [match[1], Number(match[2])]),
      );
      if (suite.status === "passed" && !suite.counts.tests) {
        suite.status = "failed";
        suite.reason =
          "Suite exited successfully but produced no TAP test counts";
      }
    }
    if (spec.id === "rust" && rustEvidence && suite.status === "passed") {
      const { captured: _captured, ...recorded } = suite;
      writeProof(rustEvidence, rustKey, "rust", { suite: recorded });
    }
    delete suite.captured;
    report.suites[spec.id] = suite;
    writeFileSync(
      join(output, "suite-progress.json"),
      JSON.stringify(report.suites, null, 2),
    );
  }
  report.items = items.map((item) =>
    evaluate(item, coverage[item.id], report.suites),
  );
  for (const item of report.items)
    report.counts[item.status] = (report.counts[item.status] ?? 0) + 1;
  report.exitCode = plan ? 0 : exitCode(report.suites, report.items);
  report.completedAt = new Date().toISOString();
  writeFileSync(
    join(output, "report.json"),
    `${JSON.stringify(report, null, 2)}\n`,
  );
  writeFileSync(join(output, "report.md"), markdown(report));
  console.log(
    `[acceptance] ${items.length} checklist items: ${JSON.stringify(report.counts)}`,
  );
  console.log(`[acceptance] Report: ${join(output, "report.md")}`);
  console.log(
    "[acceptance] Native device and live-account verification remain separate.",
  );
  return report.exitCode;
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    process.exitCode = await main();
  } catch (error) {
    console.error(`[acceptance] ${redact(error.message)}`);
    process.exitCode = 1;
  }
}
