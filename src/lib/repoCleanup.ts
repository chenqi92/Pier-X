// Shell snippets for "disable a stale third-party repo we just
// detected during install" — paired with the `repoWarnings` array on
// `SoftwareInstallReport`. Each function returns the *raw command
// text* that should be injected into the user's SSH terminal (or
// copied to the clipboard when no terminal is attached).
//
// We deliberately do NOT auto-execute these. The button hands the
// command to the active terminal without a trailing `\n` so the user
// reviews and presses Enter themselves — same UX contract as
// `sendCdToTerminal`.

/**
 * Parse one entry from `SoftwareInstallReport.repoWarnings` (format:
 * `"<manager>: <url-or-id>"`) into a shell command that disables the
 * offending repo for the duration the user keeps the comment in
 * place. Backups are always written to a `*.pierx-bak` sibling so a
 * single `mv` can roll back.
 *
 * Returns an empty string for warning shapes we don't know how to
 * clean up — the caller falls back to clipboard with the raw warning
 * text in that case.
 */
export function buildRepoCleanupCommand(warning: string): string {
  const match = /^([a-z/]+):\s*(.*)$/i.exec(warning);
  if (!match) return "";
  const manager = match[1].toLowerCase();
  const ident = match[2].trim();
  if (!ident) return "";

  if (manager === "apt") {
    // Inputs we receive from the backend look like:
    //   "https://download.docker.com/linux/ubuntu focal Release"
    //   "http://example.invalid/repo/dists/focal/InRelease"
    // We want a substring stable enough to match the original
    // `deb [...] https://download.docker.com/linux/ubuntu focal stable`
    // line in `/etc/apt/sources.list.d/*.list`. Strip scheme, strip
    // the trailing `(In)?Release` / `(In)?Release.gpg` token (the
    // suite name stays — it's the last meaningful path segment in the
    // source line itself).
    let pattern = ident
      .replace(/^https?:\/\//, "")
      .replace(/\/?dists\/[^\s]*$/i, "")
      .replace(/\s+(In)?Release(\.gpg)?\s*$/i, "")
      .trim();
    // `#` is sed's alternate-delimiter — escape it just in case some
    // private repo URL embeds one. `'` would terminate the outer
    // single-quoted sed program; we guard by using only the path part
    // (URLs don't normally carry single quotes anyway).
    pattern = pattern.replace(/#/g, "\\#").replace(/'/g, "");
    return (
      `sudo sed -i.pierx-bak '\\#${pattern}#s|^deb |# DISABLED-BY-PIERX deb |' ` +
      `/etc/apt/sources.list.d/*.list /etc/apt/sources.list 2>/dev/null; ` +
      `sudo apt-get update`
    );
  }

  if (manager === "dnf/yum" || manager === "dnf" || manager === "yum") {
    // ident is the repo id, sometimes single-quoted from the
    // backend's pattern extractor. Strip surrounding quotes.
    const id = ident.replace(/^['"]/, "").replace(/['"]$/, "");
    if (!id) return "";
    // dnf-utils ≥4 ships `config-manager --set-disabled`; older yum
    // hosts use `yum-config-manager --disable`. The `||` chain tries
    // both so the snippet works on RHEL 7, 8, 9, Fedora, and the
    // RHEL-clones (openEuler / Anolis / Kylin / TencentOS / …).
    const safe = id.replace(/'/g, "'\\''");
    return (
      `sudo dnf config-manager --set-disabled '${safe}' 2>/dev/null ` +
      `|| sudo yum-config-manager --disable '${safe}'`
    );
  }

  if (manager === "zypper") {
    const id = ident
      .replace(/^['"]/, "")
      .replace(/['"]$/, "")
      .replace(/'/g, "'\\''");
    if (!id) return "";
    return `sudo zypper modifyrepo --disable '${id}'`;
  }

  return "";
}
