#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const [reportPath, ...expectedLockfiles] = process.argv.slice(2);

if (!reportPath || expectedLockfiles.length === 0) {
  throw new Error("Usage: check-osv-severity <report.json> <lockfile>...");
}

const report = JSON.parse(readFileSync(resolve(reportPath), "utf8"));
if (!Array.isArray(report.results)) {
  throw new Error("OSV report is missing its results array");
}

const scannedPaths = report.results.map((result) => result.source?.path).filter(Boolean);
for (const lockfile of expectedLockfiles) {
  const suffix = `/${lockfile}`;
  if (!scannedPaths.some((path) => path === lockfile || path.endsWith(suffix))) {
    throw new Error(`OSV report is missing lockfile: ${lockfile}`);
  }
}

const findings = [];
for (const result of report.results) {
  for (const entry of result.packages ?? []) {
    for (const group of entry.groups ?? []) {
      const ids = Array.isArray(group.ids) ? group.ids : [];
      const rawSeverity = group.max_severity;
      const severity =
        rawSeverity === null || rawSeverity === undefined || rawSeverity === ""
          ? Number.NaN
          : Number(rawSeverity);
      const unscored = ids.length > 0 && !Number.isFinite(severity);

      if ((Number.isFinite(severity) && severity >= 7) || unscored) {
        findings.push({
          ids: ids.join(", ") || "unknown advisory",
          package: `${entry.package?.name ?? "unknown"}@${entry.package?.version ?? "unknown"}`,
          severity: unscored ? "unscored" : severity,
          source: result.source?.path ?? "unknown source",
        });
      }
    }
  }
}

console.log(
  `OSV scanned ${expectedLockfiles.length} lockfiles; blocked findings: ${findings.length}`,
);
for (const finding of findings) {
  console.error(
    `${finding.ids}: ${finding.package} (severity ${finding.severity}) in ${finding.source}`,
  );
}

if (findings.length > 0) {
  process.exitCode = 1;
}
