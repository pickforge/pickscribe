#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const script = resolve(root, "scripts/check-osv-severity.mjs");
const fixture = (name) => resolve(root, "tests/fixtures/osv", name);

function run(name) {
  return spawnSync(process.execPath, [script, fixture(name), "Cargo.lock", "bun.lock"], {
    encoding: "utf8",
  });
}

const high = run("high.json");
assert.equal(high.status, 1, high.stderr);
assert.match(high.stderr, /OSV-HIGH/);

const medium = run("medium.json");
assert.equal(medium.status, 0, medium.stderr);

const unscored = run("unscored.json");
assert.equal(unscored.status, 1, unscored.stderr);
assert.match(unscored.stderr, /OSV-UNSCORED/);
assert.match(unscored.stderr, /severity unscored/);

const missingLockfile = run("missing-lockfile.json");
assert.notEqual(missingLockfile.status, 0);
assert.match(missingLockfile.stderr, /missing lockfile: bun\.lock/);

console.log("OSV severity gate fixture tests passed");
