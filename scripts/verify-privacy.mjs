import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";

const source = readFileSync(
  new URL("../src/core/Settings.tsx", import.meta.url),
  "utf8",
);
const configured = source.match(
  /export const PRIVACY_POLICY_URL\s*=\s*["']([^"']+)["']/,
)?.[1];
const canonical =
  "https://katenorth7.github.io/desktop_buddy/docs/privacy.html";
const policy = readFileSync(
  new URL("../docs/privacy.html", import.meta.url),
  "utf8",
);
const hash = (value) =>
  createHash("sha256").update(value.replaceAll("\r\n", "\n")).digest("hex");
let failed = configured !== canonical;
for (const url of new Set([configured, canonical])) {
  if (!url) {
    failed = true;
    continue;
  }
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(30_000) });
    const body = await response.text();
    const matches = hash(body) === hash(policy);
    const externalFonts = /fonts\.googleapis\.com|fonts\.gstatic\.com/.test(
      body,
    );
    console.log(
      JSON.stringify({
        configured: url === configured,
        url,
        finalUrl: response.url,
        status: response.status,
        matchesLocalPolicy: matches,
        externalFonts,
      }),
    );
    if (response.status !== 200 || !matches || externalFonts) failed = true;
  } catch (error) {
    console.error(`${url}: ${error.message}`);
    failed = true;
  }
}
if (failed)
  console.error(
    "Privacy fix is incomplete: Developer 2 must set the canonical app URL, and the current policy and local fonts must be published through the existing Pages configuration.",
  );
process.exitCode = failed ? 1 : 0;
