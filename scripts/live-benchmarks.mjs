// Runs the real-model goal benchmarks (goal understanding and episode review) in one go
// and prints their summaries. Opt-in and never part of CI: it makes real Claude calls.
// The key comes from ANTHROPIC_API_KEY or from the ignored file
// .tools/anthropic-benchmark-key in the primary checkout; it is never printed.
import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve, join } from "node:path";

const commonGit = execFileSync("git", ["rev-parse", "--git-common-dir"], {
  encoding: "utf8",
  windowsHide: true,
}).trim();
const repository = dirname(resolve(commonGit));
const env = { ...process.env, CARGO_TERM_COLOR: "never", SUMI_LIVE_BENCHMARK: "1" };
const keyFile = join(repository, ".tools/anthropic-benchmark-key");
if (!env.ANTHROPIC_API_KEY?.trim() && existsSync(keyFile)) {
  env.ANTHROPIC_API_KEY = readFileSync(keyFile, "utf8").trim();
}
if (!env.ANTHROPIC_API_KEY?.trim()) {
  console.error(
    "No Claude API key: set ANTHROPIC_API_KEY or save it in .tools/anthropic-benchmark-key (ignored by Git).",
  );
  process.exit(2);
}
const localCargo = join(
  repository,
  ".tools/cargo/bin",
  process.platform === "win32" ? "cargo.exe" : "cargo",
);
let cargo = "cargo";
if (existsSync(localCargo)) {
  cargo = localCargo;
  env.CARGO_HOME = join(repository, ".tools/cargo");
  env.RUSTUP_HOME = join(repository, ".tools/rustup");
  env.PATH = `${dirname(cargo)}${process.platform === "win32" ? ";" : ":"}${env.PATH}`;
}
const child = spawn(
  cargo,
  [
    "test",
    "--locked",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--lib",
    "--",
    "--ignored",
    "--nocapture",
    "--test-threads=1",
    "understanding_live_benchmark",
    "episode_review_live_benchmark",
  ],
  { env, windowsHide: true },
);
let log = "";
const keep = (chunk) => {
  const text = chunk.toString();
  log += text;
  for (const line of text.split("\n")) {
    if (/: (pass|FAIL|ERROR) in |passed|skipped:|^error/.test(line)) console.log(line);
  }
};
child.stdout.on("data", keep);
child.stderr.on("data", keep);
child.on("close", (code) => {
  mkdirSync(join(repository, ".tools/quality"), { recursive: true });
  const report = join(
    repository,
    ".tools/quality",
    `live-benchmarks-${new Date().toISOString().replace(/[:.]/g, "-")}.log`,
  );
  writeFileSync(report, log.replaceAll(env.ANTHROPIC_API_KEY, "[redacted]"));
  console.log(`Full log: ${report}`);
  process.exit(code ?? 1);
});
