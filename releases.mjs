export const REPOSITORY = "KateNorth7/desktop_buddy";
export const LATEST_RELEASE = `https://github.com/${REPOSITORY}/releases/latest`;
export const RELEASE_API = `https://api.github.com/repos/${REPOSITORY}/releases/latest`;

// Only accept the exact public repository's installer URLs, never arbitrary API links.
export function selectDownloads(release) {
  if (
    !release ||
    release.draft ||
    release.prerelease ||
    !/^v\d+\.\d+\.\d+$/.test(release.tag_name)
  )
    throw new Error("No stable release");
  const version = release.tag_name.slice(1);
  const expected = {
    windows: `sumi_${version}_x64-setup.exe`,
    macos: `sumi_${version}_aarch64.dmg`,
  };
  const result = { version: release.tag_name };
  for (const [platform, name] of Object.entries(expected)) {
    const asset = release.assets?.find(
      (item) => item.name === name && item.state === "uploaded",
    );
    const url = `https://github.com/${REPOSITORY}/releases/download/${release.tag_name}/${name}`;
    if (!asset || asset.browser_download_url !== url || !(asset.size > 0))
      throw new Error(`Missing ${platform} installer`);
    result[platform] = url;
  }
  return result;
}

export function youtubeEmbed(id) {
  if (!/^[A-Za-z0-9_-]{11}$/.test(id))
    throw new Error("Invalid YouTube video ID");
  return `https://www.youtube-nocookie.com/embed/${id}?rel=0&playsinline=1&autoplay=1`;
}
