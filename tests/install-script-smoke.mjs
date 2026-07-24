import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, lstatSync, mkdirSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const repoRoot = new URL("..", import.meta.url).pathname;
const installer = join(repoRoot, "scripts", "install.sh");

function makeTempRoot(name) {
  const root = execFileSync("mktemp", ["-d", join(tmpdir(), `${name}.XXXXXX`)], {
    encoding: "utf8",
  }).trim();
  mkdirSync(join(root, "home"), { recursive: true });
  return root;
}

function writeExecutable(path, body) {
  writeFileSync(path, body, { mode: 0o755 });
}

function writeFixture(root) {
  const fixture = join(root, "fixture");
  mkdirSync(fixture, { recursive: true });
  writeFileSync(
    join(fixture, "release.json"),
    JSON.stringify({
      tag_name: "v9.9.9",
      assets: [
        { browser_download_url: "https://github.com/pickforge/pickscribe/releases/download/v9.9.9/PickScribe_9.9.9_amd64.AppImage" },
        { browser_download_url: "https://github.com/pickforge/pickscribe/releases/download/v9.9.9/PickScribe_9.9.9_aarch64.app.tar.gz" },
      ],
    }),
  );
  writeExecutable(
    join(fixture, "PickScribe_9.9.9_amd64.AppImage"),
    `#!/bin/sh
exit 0
`,
  );
  writeMacBundleFixture(fixture, "first bundle");
  return fixture;
}

function writeMacBundleFixture(fixture, marker) {
  const bundleRoot = join(fixture, "bundle");
  const bundleBinary = join(bundleRoot, "PickScribe.app", "Contents", "MacOS", "pickscribe-app");
  rmSync(bundleRoot, { recursive: true, force: true });
  mkdirSync(join(bundleRoot, "PickScribe.app", "Contents", "MacOS"), { recursive: true });
  writeExecutable(
    bundleBinary,
    `#!/bin/sh
# ${marker}
printf '%s\\n' "$@" > "$HOME/wrapper-args"
`,
  );
  execFileSync("tar", ["-czf", join(fixture, "PickScribe_9.9.9_aarch64.app.tar.gz"), "-C", bundleRoot, "PickScribe.app"]);
}

function writeFakeUname(fakebin) {
  writeExecutable(
    join(fakebin, "uname"),
    `#!/bin/sh
case "\${1:-}" in
  -s) printf '%s\\n' "$PICKSCRIBE_TEST_OS" ;;
  -m) printf '%s\\n' "$PICKSCRIBE_TEST_ARCH" ;;
  *) exit 64 ;;
esac
`,
  );
}

function writeFakeSysctl(fakebin) {
  writeExecutable(
    join(fakebin, "sysctl"),
    `#!/bin/sh
if [ "\${1:-}" = "-in" ] && [ "\${2:-}" = "sysctl.proc_translated" ]; then
  printf '%s\\n' "\${PICKSCRIBE_TEST_PROC_TRANSLATED:-0}"
  exit 0
fi
exit 64
`,
  );
}

function writeFakeCurl(fakebin) {
  writeExecutable(
    join(fakebin, "curl"),
    `#!/bin/sh
set -eu
out=""
url=""
auth_header=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      out="$2"
      shift 2
      ;;
    -H)
      case "$2" in *Authorization*) auth_header=1 ;; esac
      shift 2
      ;;
    -K|--proto|--proto-redir)
      shift 2
      ;;
    -*)
      shift
      ;;
    *)
      url="$1"
      shift
      ;;
  esac
done
case "$url" in
  *api.github.com*|*release.test*)
    if [ "$auth_header" -eq 1 ] && [ "$url" = "https://release.test/latest" ]; then
      echo "authorization header sent to release override" >&2
      exit 65
    fi
    cat "$PICKSCRIBE_TEST_FIXTURE/release.json"
    ;;
  *.AppImage|*.app.tar.gz)
    cp "$PICKSCRIBE_TEST_FIXTURE/\${url##*/}" "$out"
    ;;
  *)
    echo "unexpected url: $url" >&2
    exit 64
    ;;
esac
`,
  );
}

function runInstaller(root, fixture, extraEnv = {}) {
  const fakebin = join(root, "fakebin");
  mkdirSync(fakebin, { recursive: true });
  writeFakeCurl(fakebin);
  writeFakeUname(fakebin);
  writeFakeSysctl(fakebin);

  const env = {
    ...process.env,
    HOME: join(root, "home"),
    XDG_DATA_HOME: join(root, "home", ".local", "share"),
    PATH: `${fakebin}:${process.env.PATH}`,
    PICKSCRIBE_TEST_FIXTURE: fixture,
    PICKSCRIBE_RELEASE_API_URL: "https://release.test/latest",
    PICKSCRIBE_TEST_OS: "Linux",
    PICKSCRIBE_TEST_ARCH: "x86_64",
    ...extraEnv,
  };

  return execFileSync("sh", [installer], {
    cwd: repoRoot,
    env,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function runInstallerFailure(root, fixture, extraEnv = {}) {
  const fakebin = join(root, "fakebin");
  mkdirSync(fakebin, { recursive: true });
  writeFakeCurl(fakebin);
  writeFakeUname(fakebin);
  writeFakeSysctl(fakebin);

  const env = {
    ...process.env,
    HOME: join(root, "home"),
    XDG_DATA_HOME: join(root, "home", ".local", "share"),
    PATH: `${fakebin}:${process.env.PATH}`,
    PICKSCRIBE_TEST_FIXTURE: fixture,
    PICKSCRIBE_RELEASE_API_URL: "https://release.test/latest",
    PICKSCRIBE_TEST_OS: "Linux",
    PICKSCRIBE_TEST_ARCH: "x86_64",
    ...extraEnv,
  };

  return spawnSync("sh", [installer], {
    cwd: repoRoot,
    env,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function test(name, fn) {
  const root = makeTempRoot(`pickscribe-installer-${name.replace(/[^a-z0-9]+/gi, "-")}`);
  try {
    fn(root);
    console.log(`ok - ${name}`);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("AppImage install writes a FUSE-aware wrapper, desktop entry, and icon", (root) => {
  const fixture = writeFixture(root);
  const output = runInstaller(root, fixture);
  const home = join(root, "home");
  const appImage = join(home, ".local", "bin", "PickScribe.AppImage");
  const command = join(home, ".local", "bin", "pickscribe-app");
  const launcher = join(home, ".local", "share", "applications", "pickscribe-app.desktop");
  const icon = join(home, ".local", "share", "icons", "hicolor", "scalable", "apps", "pickscribe-app.svg");

  assert.equal(existsSync(appImage), true);
  assert.equal(statSync(appImage).mode & 0o111, 0o111);
  assert.match(readFileSync(command, "utf8"), /APPIMAGE_EXTRACT_AND_RUN=1/);
  assert.match(readFileSync(command, "utf8"), new RegExp(appImage.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  assert.match(readFileSync(launcher, "utf8"), /Exec=".*\/\.local\/bin\/pickscribe-app"/);
  assert.match(readFileSync(launcher, "utf8"), /Icon=pickscribe-app/);
  assert.match(readFileSync(launcher, "utf8"), /^StartupWMClass=Pickscribe-app$/m);
  assert.equal(existsSync(icon), true);
  assert.match(output, /Launch with `pickscribe-app`/);
});

test("macOS install extracts the app and writes an argument-forwarding wrapper", (root) => {
  const fixture = writeFixture(root);
  const output = runInstaller(root, fixture, {
    PICKSCRIBE_TEST_OS: "Darwin",
    PICKSCRIBE_TEST_ARCH: "arm64",
  });
  const home = join(root, "home");
  const app = join(home, "Applications", "PickScribe.app");
  const binary = join(app, "Contents", "MacOS", "pickscribe-app");
  const command = join(home, ".local", "bin", "pickscribe-app");

  assert.equal(existsSync(binary), true);
  assert.match(readFileSync(command, "utf8"), new RegExp(binary.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  execFileSync(command, ["--toggle"], { env: { ...process.env, HOME: home } });
  assert.equal(readFileSync(join(home, "wrapper-args"), "utf8"), "--toggle\n");
  assert.match(output, /PickScribe v9\.9\.9 installed to .*\/Applications\/PickScribe\.app/);

  const firstBinary = readFileSync(binary, "utf8");
  writeMacBundleFixture(fixture, "replacement bundle");
  runInstaller(root, fixture, {
    PICKSCRIBE_TEST_OS: "Darwin",
    PICKSCRIBE_TEST_ARCH: "arm64",
  });
  const replacementBinary = readFileSync(binary, "utf8");
  assert.equal(existsSync(binary), true);
  assert.match(firstBinary, /first bundle/);
  assert.match(replacementBinary, /replacement bundle/);
  assert.notEqual(replacementBinary, firstBinary);
});

test("macOS Intel install fails with an Apple silicon-only message", (root) => {
  const fixture = writeFixture(root);
  const result = runInstallerFailure(root, fixture, {
    PICKSCRIBE_TEST_OS: "Darwin",
    PICKSCRIBE_TEST_ARCH: "x86_64",
    PICKSCRIBE_TEST_PROC_TRANSLATED: "0",
  });

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /currently ships Apple silicon \(aarch64\) only/);
});

test("macOS Rosetta install selects the Apple silicon bundle", (root) => {
  const fixture = writeFixture(root);
  runInstaller(root, fixture, {
    PICKSCRIBE_TEST_OS: "Darwin",
    PICKSCRIBE_TEST_ARCH: "x86_64",
    PICKSCRIBE_TEST_PROC_TRANSLATED: "1",
  });

  assert.equal(
    existsSync(join(root, "home", "Applications", "PickScribe.app", "Contents", "MacOS", "pickscribe-app")),
    true,
  );
});

test("macOS wrapper handles spaces and single quotes in HOME", (root) => {
  const fixture = writeFixture(root);
  const home = join(root, "home with a space's quote");
  mkdirSync(home, { recursive: true });
  runInstaller(root, fixture, {
    HOME: home,
    XDG_DATA_HOME: join(home, ".local", "share"),
    PICKSCRIBE_TEST_OS: "Darwin",
    PICKSCRIBE_TEST_ARCH: "arm64",
  });

  const command = join(home, ".local", "bin", "pickscribe-app");
  execFileSync(command, ["argument with space", "quote's intact"], { env: { ...process.env, HOME: home } });
  assert.equal(readFileSync(join(home, "wrapper-args"), "utf8"), "argument with space\nquote's intact\n");
});

test("AppImage upgrade replaces old symlink command without overwriting the AppImage", (root) => {
  const fixture = writeFixture(root);
  const bin = join(root, "home", ".local", "bin");
  mkdirSync(bin, { recursive: true });
  writeExecutable(join(bin, "PickScribe.AppImage"), "#!/bin/sh\nexit 0\n");
  symlinkSync("PickScribe.AppImage", join(bin, "pickscribe-app"));

  runInstaller(root, fixture);

  const appImage = readFileSync(join(bin, "PickScribe.AppImage"), "utf8");
  const command = readFileSync(join(bin, "pickscribe-app"), "utf8");

  assert.equal(lstatSync(join(bin, "pickscribe-app")).isSymbolicLink(), false);
  assert.doesNotMatch(appImage, /APPIMAGE_EXTRACT_AND_RUN=1/);
  assert.match(command, /APPIMAGE_EXTRACT_AND_RUN=1/);
});

test("release API override does not receive the GitHub token", (root) => {
  const fixture = writeFixture(root);

  const output = runInstaller(root, fixture, { GITHUB_TOKEN: "ghp_secret" });

  assert.match(output, /PickScribe v9\.9\.9 installed/);
});

test("AppImage install refuses to overwrite an unrelated command", (root) => {
  const fixture = writeFixture(root);
  const bin = join(root, "home", ".local", "bin");
  mkdirSync(bin, { recursive: true });
  writeExecutable(join(bin, "pickscribe-app"), "#!/bin/sh\nexit 0\n");

  const result = runInstallerFailure(root, fixture);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /command path already exists and was not created by PickScribe/);
});
