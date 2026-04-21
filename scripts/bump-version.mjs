#!/usr/bin/env node
// Bump Pier-X version across the frontend, the Tauri config, and both
// Cargo manifests. Optionally stages, commits, and tags the change.
//
// Usage:
//   node scripts/bump-version.mjs <version>     (e.g. 0.2.0)
//   node scripts/bump-version.mjs major|minor|patch
//   node scripts/bump-version.mjs <version> --no-git
//   node scripts/bump-version.mjs <version> --no-tag
//   node scripts/bump-version.mjs <version> --push     (git push + push --tags)
//   node scripts/bump-version.mjs <version> --dry-run

import { readFileSync, writeFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { execSync } from "node:child_process";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "..");

const TARGETS = [
  {
    path: resolve(repoRoot, "package.json"),
    kind: "json",
    key: "version",
  },
  {
    path: resolve(repoRoot, "src-tauri/tauri.conf.json"),
    kind: "json",
    key: "version",
  },
  {
    path: resolve(repoRoot, "src-tauri/Cargo.toml"),
    kind: "cargo",
  },
  {
    path: resolve(repoRoot, "pier-core/Cargo.toml"),
    kind: "cargo",
  },
];

const SEMVER_RE = /^(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?$/;

function die(msg) {
  console.error(`bump-version: ${msg}`);
  process.exit(1);
}

function parseArgs(argv) {
  const flags = { noGit: false, noTag: false, dryRun: false, push: false };
  const positional = [];
  for (const a of argv) {
    if (a === "--no-git") flags.noGit = true;
    else if (a === "--no-tag") flags.noTag = true;
    else if (a === "--dry-run") flags.dryRun = true;
    else if (a === "--push") flags.push = true;
    else if (a.startsWith("--")) die(`unknown flag: ${a}`);
    else positional.push(a);
  }
  if (positional.length !== 1) {
    die("expected one version argument (e.g. 0.2.0, or major|minor|patch)");
  }
  if (flags.push && flags.noGit) {
    die("--push conflicts with --no-git");
  }
  return { arg: positional[0], flags };
}

function readCurrentVersion() {
  const pkg = JSON.parse(readFileSync(TARGETS[0].path, "utf8"));
  if (!pkg.version || !SEMVER_RE.test(pkg.version)) {
    die(`unable to read current version from package.json (got: ${pkg.version})`);
  }
  return pkg.version;
}

function resolveTarget(arg, current) {
  if (["major", "minor", "patch"].includes(arg)) {
    const m = current.match(SEMVER_RE);
    if (!m) die(`current version ${current} is not valid semver`);
    let [_, maj, min, pat] = m;
    maj = Number(maj); min = Number(min); pat = Number(pat);
    if (arg === "major") { maj += 1; min = 0; pat = 0; }
    else if (arg === "minor") { min += 1; pat = 0; }
    else { pat += 1; }
    return `${maj}.${min}.${pat}`;
  }
  if (!SEMVER_RE.test(arg)) die(`invalid version: ${arg}`);
  return arg;
}

function updateJson(target, next) {
  const raw = readFileSync(target.path, "utf8");
  const obj = JSON.parse(raw);
  if (obj[target.key] === undefined) {
    die(`${target.path} is missing key "${target.key}"`);
  }
  obj[target.key] = next;
  // Preserve trailing newline if the original had one.
  const trailing = raw.endsWith("\n") ? "\n" : "";
  writeFileSync(target.path, JSON.stringify(obj, null, 2) + trailing);
}

function updateCargo(target, next) {
  const raw = readFileSync(target.path, "utf8");
  // Only touch the first `version = "..."` line under [package].
  // Keeping this narrow avoids mutating dependency version pins.
  const packageSection = raw.match(/^\[package\][\s\S]*?(?=^\[|\Z)/m);
  if (!packageSection) die(`${target.path} has no [package] section`);
  const section = packageSection[0];
  const versionLine = section.match(/^version\s*=\s*"([^"]+)"\s*$/m);
  if (!versionLine) die(`${target.path} [package] has no version line`);
  const updatedSection = section.replace(
    /^version\s*=\s*"[^"]+"\s*$/m,
    `version = "${next}"`,
  );
  writeFileSync(target.path, raw.replace(section, updatedSection));
}

function runGit(cmd, dryRun) {
  if (dryRun) {
    console.log(`[dry-run] ${cmd}`);
    return;
  }
  execSync(cmd, { cwd: repoRoot, stdio: "inherit" });
}

function main() {
  const { arg, flags } = parseArgs(process.argv.slice(2));
  const current = readCurrentVersion();
  const next = resolveTarget(arg, current);
  if (next === current) die(`version is already ${current}; nothing to do`);

  console.log(`Bumping ${current} -> ${next}`);
  for (const t of TARGETS) {
    console.log(`  update ${t.path}`);
    if (flags.dryRun) continue;
    if (t.kind === "json") updateJson(t, next);
    else updateCargo(t, next);
  }

  if (flags.noGit) return;

  const files = TARGETS.map((t) => t.path).map((p) => `"${p}"`).join(" ");
  runGit(`git add ${files}`, flags.dryRun);
  runGit(`git commit -m "chore(release): v${next}"`, flags.dryRun);
  if (!flags.noTag) {
    runGit(`git tag -a v${next} -m "Pier-X v${next}"`, flags.dryRun);
    if (flags.push) {
      runGit(`git push`, flags.dryRun);
      runGit(`git push origin v${next}`, flags.dryRun);
      console.log(`\nPushed v${next} to origin.`);
    } else {
      console.log(`\nTagged v${next}. Push with:`);
      console.log(`  git push && git push --tags`);
    }
  }
}

main();
