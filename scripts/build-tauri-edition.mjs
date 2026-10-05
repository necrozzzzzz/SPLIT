import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const [edition, output = "app"] = process.argv.slice(2);

if (edition !== "borderless" && edition !== "fullscreen") {
  console.error("Edition must be borderless or fullscreen.");
  process.exit(2);
}

if (output !== "app" && output !== "bundle") {
  console.error("Output must be app or bundle.");
  process.exit(2);
}

const root = resolve(import.meta.dirname, "..");
const tauriDirectory = resolve(root, "src-tauri");
const baseConfigPath = resolve(tauriDirectory, "tauri.conf.json");
const editionConfigPath = resolve(
  tauriDirectory,
  `tauri.${edition}.conf.json`,
);
const baseConfig = JSON.parse(readFileSync(baseConfigPath, "utf8"));
const editionConfig = JSON.parse(readFileSync(editionConfigPath, "utf8"));
const effectiveConfig = {
  ...baseConfig,
  ...editionConfig,
};

const expectedIdentity = {
  borderless: {
    productName: "SPLIT Borderless",
    mainBinaryName: "SPLIT-Borderless",
    identifier: "com.necrozzzzzz.split.borderless",
  },
  fullscreen: {
    productName: "SPLIT Fullscreen",
    mainBinaryName: "SPLIT-Fullscreen",
    identifier: "com.necrozzzzzz.split.fullscreen",
  },
}[edition];

for (const [key, expected] of Object.entries(expectedIdentity)) {
  if (effectiveConfig[key] !== expected) {
    console.error(
      `Effective Tauri config for ${edition}: expected ${key}=${JSON.stringify(expected)}`,
    );
    process.exit(2);
  }
}

const npmCli = process.env.npm_execpath;
if (!npmCli) {
  console.error("This script must be launched through an npm script.");
  process.exit(2);
}

const tauriArgs = [
  npmCli,
  "run",
  "tauri",
  "--",
  "build",
  "--config",
  editionConfigPath,
];

if (output === "app") {
  tauriArgs.push("--no-bundle");
} else {
  tauriArgs.push("--bundles", "nsis");
}

const buildStartedAt = Date.now();
const result = spawnSync(process.execPath, tauriArgs, {
  cwd: root,
  env: {
    ...process.env,
    SPLIT_EDITION: edition,
  },
  stdio: "inherit",
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

if (result.status !== 0) {
  process.exit(result.status ?? 1);
}

const artifactDirectory = resolve(root, "artifacts", edition);
mkdirSync(artifactDirectory, { recursive: true });

const executableName = `${effectiveConfig.mainBinaryName}.exe`;
const executableSource = resolve(
  tauriDirectory,
  "target",
  "release",
  executableName,
);
const executableTarget = resolve(artifactDirectory, executableName);
copyFileSync(executableSource, executableTarget);

const expectedInstallerName =
  `${effectiveConfig.mainBinaryName}-Setup-${baseConfig.version}.exe`;
let installerName = existsSync(
  resolve(artifactDirectory, expectedInstallerName),
)
  ? expectedInstallerName
  : null;
if (output === "bundle") {
  const nsisDirectory = resolve(
    tauriDirectory,
    "target",
    "release",
    "bundle",
    "nsis",
  );
  const installerCandidates = readdirSync(nsisDirectory)
    .filter((name) => name.endsWith("-setup.exe"))
    .map((name) => ({
      name,
      modifiedAt: statSync(resolve(nsisDirectory, name)).mtimeMs,
    }))
    .filter(({ modifiedAt }) => modifiedAt >= buildStartedAt - 2_000)
    .sort((left, right) => right.modifiedAt - left.modifiedAt);

  if (installerCandidates.length === 0) {
    console.error("Tauri did not produce a fresh NSIS installer.");
    process.exit(1);
  }

  installerName = expectedInstallerName;
  copyFileSync(
    resolve(nsisDirectory, installerCandidates[0].name),
    resolve(artifactDirectory, installerName),
  );
}

writeFileSync(
  resolve(artifactDirectory, "build-manifest.json"),
  `${JSON.stringify(
    {
      edition,
      productName: effectiveConfig.productName,
      identifier: effectiveConfig.identifier,
      version: baseConfig.version,
      executable: executableName,
      installer: installerName,
    },
    null,
    2,
  )}\n`,
);

console.log(`Edition artifact: ${executableTarget}`);
if (installerName) {
  console.log(`Installer artifact: ${resolve(artifactDirectory, installerName)}`);
}
