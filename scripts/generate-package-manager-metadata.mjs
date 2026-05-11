#!/usr/bin/env node
// Generate package-manager metadata from a published Pier-X release.
//
// The script expects release assets to already be present locally so the
// checksums in the Homebrew cask and WinGet manifests are always computed
// from the exact files users will download.

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";

const DEFAULT_REPO = "chenqi92/Pier-X";
const MANIFEST_VERSION = "1.12.0";
const PACKAGE_IDENTIFIER = "Chenqi92.PierX";

function die(msg) {
  console.error(`generate-package-manager-metadata: ${msg}`);
  process.exit(1);
}

function parseArgs(argv) {
  const opts = {
    version: "",
    repo: DEFAULT_REPO,
    assetsDir: "",
    outDir: "dist-package-managers",
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = () => {
      i += 1;
      if (i >= argv.length) die(`missing value for ${arg}`);
      return argv[i];
    };

    if (arg === "--version") opts.version = next();
    else if (arg === "--repo") opts.repo = next();
    else if (arg === "--assets-dir") opts.assetsDir = next();
    else if (arg === "--out-dir") opts.outDir = next();
    else if (arg === "--help" || arg === "-h") {
      console.log(`Usage:
  node scripts/generate-package-manager-metadata.mjs \\
    --version 0.3.0 \\
    --assets-dir release-assets \\
    --out-dir dist-package-managers

Options:
  --repo       GitHub owner/repo for release URLs (default: ${DEFAULT_REPO})
  --out-dir    Output directory (default: dist-package-managers)`);
      process.exit(0);
    } else {
      die(`unknown argument: ${arg}`);
    }
  }

  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(opts.version)) {
    die("--version must be a valid semver string such as 0.3.0");
  }
  if (!opts.assetsDir) die("--assets-dir is required");
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(opts.repo)) {
    die("--repo must be in owner/name form");
  }

  return {
    ...opts,
    assetsDir: resolve(opts.assetsDir),
    outDir: resolve(opts.outDir),
  };
}

function sha256(path) {
  const hash = createHash("sha256");
  hash.update(readFileSync(path));
  return hash.digest("hex");
}

function requireAsset(assetsDir, name) {
  const path = join(assetsDir, name);
  if (!existsSync(path)) die(`missing release asset: ${path}`);
  return {
    name,
    path,
    sha256: sha256(path),
  };
}

function releaseUrl(repo, version, assetName) {
  return `https://github.com/${repo}/releases/download/v${version}/${encodeURIComponent(assetName)}`;
}

function writeFile(path, content) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content.trimEnd() + "\n");
}

function yamlString(value) {
  return JSON.stringify(value);
}

function generateHomebrew({ outDir, repo, version, macDmg, linuxAppImage }) {
  const url = releaseUrl(repo, version, macDmg.name);
  const cask = `cask "pier-x" do
  version "${version}"
  sha256 "${macDmg.sha256}"

  url "${url}"
  name "Pier-X"
  desc "IDE-style desktop workbench for terminal, Git, SSH, databases, and remote ops"
  homepage "https://github.com/${repo}"

  livecheck do
    url "https://github.com/${repo}/releases/latest"
    strategy :github_latest
  end

  app "Pier-X.app"

  zap trash: [
    "~/Library/Application Support/com.kkape.pierx",
    "~/Library/Preferences/com.kkape.pierx.plist",
    "~/Library/Saved Application State/com.kkape.pierx.savedState",
  ]
end`;

  writeFile(join(outDir, "homebrew", "Casks", "pier-x.rb"), cask);

  const formulaUrl = releaseUrl(repo, version, linuxAppImage.name);
  const formula = `class PierX < Formula
  desc "IDE-style desktop workbench for terminal, Git, SSH, databases, and remote ops"
  homepage "https://github.com/${repo}"
  url "${formulaUrl}"
  sha256 "${linuxAppImage.sha256}"
  license "MIT"

  depends_on :linux

  def install
    bin.install "${linuxAppImage.name}" => "pier-x"
    chmod 0755, bin/"pier-x"
  end

  test do
    assert_predicate bin/"pier-x", :exist?
  end
end`;

  writeFile(join(outDir, "homebrew", "Formula", "pier-x.rb"), formula);
}

function wingetHeader({ version, manifestType }) {
  return `# yaml-language-server: $schema=https://aka.ms/winget-manifest.${manifestType}.${MANIFEST_VERSION}.schema.json
PackageIdentifier: ${PACKAGE_IDENTIFIER}
PackageVersion: ${version}
ManifestType: ${manifestType}
ManifestVersion: ${MANIFEST_VERSION}`;
}

function generateWinget({ outDir, repo, version, winX64, winArm64 }) {
  const base = join(
    outDir,
    "winget",
    "manifests",
    "c",
    "Chenqi92",
    "PierX",
    version,
  );

  const versionManifest = `${wingetHeader({ version, manifestType: "version" })}
DefaultLocale: en-US`;

  const installerManifest = `${wingetHeader({ version, manifestType: "installer" })}
InstallerType: nullsoft
Installers:
- Architecture: x64
  InstallerUrl: ${yamlString(releaseUrl(repo, version, winX64.name))}
  InstallerSha256: ${winX64.sha256.toUpperCase()}
- Architecture: arm64
  InstallerUrl: ${yamlString(releaseUrl(repo, version, winArm64.name))}
  InstallerSha256: ${winArm64.sha256.toUpperCase()}`;

  const defaultLocaleManifest = `${wingetHeader({ version, manifestType: "defaultLocale" })}
PackageLocale: en-US
Publisher: chenqi92
PublisherUrl: ${yamlString("https://github.com/chenqi92")}
PublisherSupportUrl: ${yamlString(`https://github.com/${repo}/issues`)}
Author: chenqi92
PackageName: Pier-X
PackageUrl: ${yamlString(`https://github.com/${repo}`)}
License: MIT
LicenseUrl: ${yamlString(`https://github.com/${repo}/blob/main/LICENSE`)}
Copyright: Copyright (c) 2026 kkape.com
ShortDescription: IDE-style desktop workbench for terminal, Git, SSH, databases, and remote ops.
Description: Pier-X is a cross-platform desktop workbench for backend and operations engineers, combining terminal, Git, SSH, databases, Docker, logs, SFTP, and server management in one Tauri-based app.
Moniker: pier-x
Tags:
- tauri
- terminal
- ssh
- git
- database
- devtools`;

  const zhLocaleManifest = `${wingetHeader({ version, manifestType: "locale" })}
PackageLocale: zh-CN
Publisher: chenqi92
PackageName: Pier-X
ShortDescription: 把终端、Git、SSH、数据库和远程运维放进一个 IDE 风格工作台的桌面工具。
Description: Pier-X 是面向后端和运维工程师的跨平台桌面工作台，整合终端、Git、SSH、数据库、Docker、日志、SFTP 和服务器管理能力。`;

  writeFile(join(base, `${PACKAGE_IDENTIFIER}.yaml`), versionManifest);
  writeFile(join(base, `${PACKAGE_IDENTIFIER}.installer.yaml`), installerManifest);
  writeFile(join(base, `${PACKAGE_IDENTIFIER}.locale.en-US.yaml`), defaultLocaleManifest);
  writeFile(join(base, `${PACKAGE_IDENTIFIER}.locale.zh-CN.yaml`), zhLocaleManifest);
}

function generateChecksums({ outDir, assets }) {
  const lines = assets
    .map((asset) => `${asset.sha256}  ${basename(asset.path)}`)
    .sort()
    .join("\n");
  writeFile(join(outDir, "SHA256SUMS.txt"), lines);
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  const { version, assetsDir } = opts;

  const macDmg = requireAsset(assetsDir, `Pier-X_${version}_universal.dmg`);
  const linuxAppImage = requireAsset(assetsDir, `Pier-X_${version}_amd64.AppImage`);
  const winX64 = requireAsset(assetsDir, `Pier-X_${version}_x64-setup.exe`);
  const winArm64 = requireAsset(assetsDir, `Pier-X_${version}_arm64-setup.exe`);

  mkdirSync(opts.outDir, { recursive: true });
  generateHomebrew({ ...opts, macDmg, linuxAppImage });
  generateWinget({ ...opts, winX64, winArm64 });
  generateChecksums({
    outDir: opts.outDir,
    assets: [macDmg, linuxAppImage, winX64, winArm64],
  });

  console.log(`Generated package-manager metadata in ${opts.outDir}`);
}

main();
