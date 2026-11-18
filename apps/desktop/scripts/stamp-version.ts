// Stamps a release's version into the Cargo workspace, which every crate takes its version from: the server's
// VERSION (CARGO_PKG_VERSION) is the app's version, and the Windows exe's details carry it. The checkout keeps
// 0.0.0; only the release workflow's copy is stamped.
//
//   node scripts/stamp-version.ts 2026.1118.1430
import * as fs from "node:fs";
import * as path from "node:path";

const REPO = path.join(import.meta.dirname, "..", "..", "..");

/** Cargo.toml with `version` under [workspace.package] set. */
export function stampCargoToml(text: string, version: string): string {
  const stamped = text.replace(/(\[workspace\.package\][^[]*?\nversion = )"[^"]*"/, `$1"${version}"`);
  if (stamped === text && !text.includes(`version = "${version}"`)) throw new Error("Cargo.toml has no [workspace.package] version.");
  return stamped;
}

/** Cargo.lock with the version of every package named in `names` set, so `cargo build --locked` still works. */
export function stampCargoLock(text: string, names: string[], version: string): string {
  return text.replace(/\[\[package\]\]\nname = "([^"]+)"\nversion = "[^"]*"/g, (entry, name: string) =>
    names.includes(name) ? `[[package]]\nname = "${name}"\nversion = "${version}"` : entry,
  );
}

if (import.meta.main) {
  const [version] = process.argv.slice(2);
  if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
    console.error("Usage: node scripts/stamp-version.ts <major.minor.patch>");
    process.exit(1);
  }
  const cargoToml = path.join(REPO, "Cargo.toml");
  fs.writeFileSync(cargoToml, stampCargoToml(fs.readFileSync(cargoToml, "utf8"), version));
  // The workspace's crates: each crates/<name>/Cargo.toml's package name.
  const names = fs
    .readdirSync(path.join(REPO, "crates"))
    .map((dir) => path.join(REPO, "crates", dir, "Cargo.toml"))
    .filter((file) => fs.existsSync(file))
    .map((file) => /\[package\]\nname = "([^"]+)"/.exec(fs.readFileSync(file, "utf8"))?.[1])
    .filter((name): name is string => Boolean(name));
  const cargoLock = path.join(REPO, "Cargo.lock");
  fs.writeFileSync(cargoLock, stampCargoLock(fs.readFileSync(cargoLock, "utf8"), names, version));
  console.log(`Stamped ${version} into Cargo.toml and Cargo.lock for ${names.join(", ")}`);
}
