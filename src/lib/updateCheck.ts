// ── Update check helper ─────────────────────────────────────────
//
// Pier-X runs strictly offline by default (see PRODUCT-SPEC §1.1).
// This module makes a single outbound HTTPS call to GitHub Releases
// — only when the user explicitly opts in (setting toggle) or
// explicitly clicks "Check for updates now". There is no silent
// auto-download and no auto-install: the check resolves to a
// structured result, and the caller decides whether to surface a
// notification or open the release page in the system browser.

const RELEASES_ENDPOINT =
  "https://api.github.com/repos/chenqi92/Pier-X/releases/latest";

export const RELEASES_PAGE = "https://github.com/chenqi92/Pier-X/releases";

export type UpdateCheckResult = {
  currentVersion: string;
  latestVersion: string;
  hasUpdate: boolean;
  releaseUrl: string;
  releaseName: string;
  publishedAt: string;
};

type GithubReleaseShape = {
  tag_name?: string;
  name?: string;
  html_url?: string;
  published_at?: string;
  draft?: boolean;
  prerelease?: boolean;
};

/**
 * Parse a semver tag like `v0.4.1`, `0.4.1`, or `v0.4.1-rc.1` into
 * a comparable tuple. Unknown / malformed strings produce
 * `[0, 0, 0, 0]`, which makes them compare equal — we'd rather
 * treat a garbled tag as "no update" than wrongly promote one.
 *
 * The 4th element captures a pre-release drop: `1.0.0-rc.1` sorts
 * strictly before `1.0.0` because its pre tag is non-zero.
 */
function parseVersion(input: string): [number, number, number, number] {
  const raw = input.trim().replace(/^v/i, "");
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-(.+))?$/.exec(raw);
  if (!match) return [0, 0, 0, 0];
  const [, major, minor, patch, pre] = match;
  // Stable > any pre-release: stable gets a big "pre index" so it
  // compares greater. Pre-releases get 0 here so the hex check
  // below orders them below stable.
  const preIndex = pre ? 0 : Number.MAX_SAFE_INTEGER;
  return [
    Number.parseInt(major, 10),
    Number.parseInt(minor, 10),
    Number.parseInt(patch, 10),
    preIndex,
  ];
}

function compareVersions(a: string, b: string): number {
  const aa = parseVersion(a);
  const bb = parseVersion(b);
  for (let i = 0; i < 4; i++) {
    if (aa[i] !== bb[i]) return aa[i] - bb[i];
  }
  return 0;
}

/**
 * Fetch the latest GitHub release for Pier-X and return a
 * structured comparison against `currentVersion`. Rejects with
 * `Error` on network failure or unexpected payload — callers
 * should show that to the user via toast.
 *
 * `signal` lets the caller cancel a long-running check (e.g. when
 * the settings dialog closes mid-request).
 */
export async function checkForUpdates(
  currentVersion: string,
  signal?: AbortSignal,
): Promise<UpdateCheckResult> {
  const response = await fetch(RELEASES_ENDPOINT, {
    signal,
    headers: { Accept: "application/vnd.github+json" },
  });
  if (!response.ok) {
    throw new Error(`GitHub returned HTTP ${response.status}`);
  }
  const payload = (await response.json()) as GithubReleaseShape;
  if (!payload.tag_name) {
    throw new Error("latest release is missing a tag name");
  }
  if (payload.draft) {
    throw new Error("latest release is a draft; ignoring");
  }
  const latestVersion = payload.tag_name;
  return {
    currentVersion,
    latestVersion,
    hasUpdate: compareVersions(latestVersion, currentVersion) > 0,
    releaseUrl: payload.html_url ?? RELEASES_PAGE,
    releaseName: payload.name ?? payload.tag_name,
    publishedAt: payload.published_at ?? "",
  };
}
