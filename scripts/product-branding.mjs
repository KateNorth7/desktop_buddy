// Published compatibility assets keep their original names; new releases use sumi.
export function assetPrefix(version) {
  if (!/^\d+\.\d+\.\d+$/.test(version))
    throw new Error("Invalid asset version");
  const [major, minor, patch] = version.split(".").map(Number);
  return major > 0 || minor > 49 || (minor === 49 && patch >= 2)
    ? "sumi"
    : "Desktop.Buddy";
}

// Releases are published from this GitHub repository; installed apps accept updates only from it.
export const RELEASE_REPOSITORY = "KateNorth7/desktop_buddy";
export function releaseDownloadUrl(tag, assetName) {
  return `https://github.com/${RELEASE_REPOSITORY}/releases/download/${tag}/${assetName}`;
}
export function updateChannelUrl(channel) {
  return `https://github.com/${RELEASE_REPOSITORY}/releases/latest/download/${channel}`;
}
