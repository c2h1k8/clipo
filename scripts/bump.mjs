#!/usr/bin/env node
// バージョン番号を package.json / tauri.conf.json / Cargo.toml / Cargo.lock でそろえる
//   npm run bump 0.2.0       … 指定したバージョンにする
//   npm run bump patch|minor|major … いまの番号から上げる
import { readFileSync, writeFileSync } from "node:fs";

const arg = process.argv[2];
const files = {
  pkg: "package.json",
  conf: "src-tauri/tauri.conf.json",
  cargo: "src-tauri/Cargo.toml",
  lock: "src-tauri/Cargo.lock",
};
const current = JSON.parse(readFileSync(files.conf, "utf8")).version;

function next(v, kind) {
  const [a, b, c] = v.split(".").map(Number);
  if (kind === "major") return `${a + 1}.0.0`;
  if (kind === "minor") return `${a}.${b + 1}.0`;
  if (kind === "patch") return `${a}.${b}.${c + 1}`;
  return null;
}

const version = next(current, arg) ?? arg;
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  console.error(`使い方: npm run bump <x.y.z | patch | minor | major>（いまは ${current}）`);
  process.exit(1);
}

const json = (path, edit) => {
  const data = JSON.parse(readFileSync(path, "utf8"));
  edit(data);
  writeFileSync(path, JSON.stringify(data, null, 2) + "\n");
};
json(files.pkg, (d) => (d.version = version));
json(files.conf, (d) => (d.version = version));

const cargo = readFileSync(files.cargo, "utf8").replace(/^version = ".*"$/m, `version = "${version}"`);
writeFileSync(files.cargo, cargo);
const lock = readFileSync(files.lock, "utf8").replace(
  /(\[\[package\]\]\nname = "clipo"\nversion = )".*"/,
  `$1"${version}"`,
);
writeFileSync(files.lock, lock);

console.log(`${current} → ${version}`);
console.log(`次: git switch -c release/v${version} && git commit -am "Release v${version}" && gh pr create --fill`);
