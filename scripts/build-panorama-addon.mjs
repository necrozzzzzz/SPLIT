import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import {
  copyFile,
  mkdir,
  readFile,
  readdir,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { homedir } from "node:os";
import { basename, dirname, extname, join, relative, resolve, sep } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "..");
const addonRoot = join(repoRoot, "panorama-addon");
const sourceRoot = join(addonRoot, "panorama");
const integrationLayout = join(sourceRoot, "layout", "split_quick_access.xml");
const stockLayout = join(
  addonRoot,
  "stock",
  "panorama",
  "layout",
  "citadel_hud_active_player_stats.xml",
);
const buildRoot = join(addonRoot, "build");
const payloadRoot = join(buildRoot, "payload");
const outputVpk = join(repoRoot, "artifacts", "panorama", "SPLIT-Panorama.vpk");
const stagingName = "build_split_panorama";

const stockSha256 = "5ae3a08a097a35c52dad422a83c06a5c610c96827865ea24022dbd72f241b8e4";
const expectedFiles = [
  "panorama/layout/citadel_hud_active_player_stats.vxml_c",
  "panorama/scripts/split_bootstrap.vjs_c",
  "panorama/scripts/split_bridge.vjs_c",
  "panorama/scripts/split_quick_access.vjs_c",
  "panorama/scripts/split_state.vjs_c",
  "panorama/scripts/split_utils.vjs_c",
  "panorama/styles/split_quick_access.vcss_c",
].sort();

const sourceFiles = [
  "scripts/split_utils.js",
  "scripts/split_state.js",
  "scripts/split_quick_access.js",
  "scripts/split_bridge.js",
  "scripts/split_bootstrap.js",
  "styles/split_quick_access.css",
];

const stockClientVersion = "6745";
const stockSourceRevision = "11078118";
const expectedStyleIncludes = ["s2r://panorama/styles/split_quick_access.vcss_c"];
const expectedScriptIncludes = [
  "s2r://panorama/scripts/split_utils.vjs_c",
  "s2r://panorama/scripts/split_state.vjs_c",
  "s2r://panorama/scripts/split_quick_access.vjs_c",
  "s2r://panorama/scripts/split_bridge.vjs_c",
  "s2r://panorama/scripts/split_bootstrap.vjs_c",
];

function fail(message) {
  throw new Error(message);
}

function normalized(path) {
  return resolve(path).toLowerCase();
}

function assertChild(parent, child, label) {
  const parentPath = `${normalized(parent)}${sep}`;
  const childPath = normalized(child);
  if (!childPath.startsWith(parentPath)) {
    fail(`Refusing to use ${label} outside ${parent}: ${child}`);
  }
}

async function cleanDirectory(parent, target, label) {
  assertChild(parent, target, label);
  await rm(target, { recursive: true, force: true });
  await mkdir(target, { recursive: true });
}

function findCsdkRoot() {
  const candidates = [
    process.env.SPLIT_CSDK_ROOT,
    process.env.DEADLOCK_CSDK_ROOT,
    join(homedir(), "Downloads", "Reduced_CSDK_12", "Reduced_CSDK_12"),
    join(homedir(), "Downloads", "Reduced_CSDK_12"),
  ].filter(Boolean);
  for (const candidate of candidates) {
    const root = resolve(candidate);
    if (
      existsSync(join(root, "game", "bin_cs2", "win64", "resourcecompiler.exe")) &&
      existsSync(join(root, "game", "bin", "win64", "CSDKCfgVPK.exe")) &&
      existsSync(join(root, "game", "citadel", "gameinfo.gi"))
    ) {
      return root;
    }
  }
  fail(
    "Reduced CSDK not found. Set SPLIT_CSDK_ROOT to a CSDK root containing " +
      "game/bin_cs2/win64/resourcecompiler.exe and game/bin/win64/CSDKCfgVPK.exe.",
  );
}

async function findDeadlockRoot() {
  const steamRoots = [join("C:", "Program Files (x86)", "Steam")];
  const libraryFile = join(steamRoots[0], "steamapps", "libraryfolders.vdf");
  if (existsSync(libraryFile)) {
    const libraries = await readFile(libraryFile, "utf8");
    for (const match of libraries.matchAll(/"path"\s+"([^"]+)"/g)) {
      steamRoots.push(match[1].replaceAll("\\\\", "\\"));
    }
  }
  const candidates = [
    process.env.SPLIT_DEADLOCK_ROOT,
    process.env.DEADLOCK_ROOT,
    ...steamRoots.map((root) => join(root, "steamapps", "common", "Deadlock")),
  ].filter(Boolean);
  for (const candidate of candidates) {
    const root = resolve(candidate);
    if (
      existsSync(join(root, "game", "citadel", "pak01_dir.vpk")) &&
      existsSync(join(root, "game", "citadel", "steam.inf"))
    ) {
      return root;
    }
  }
  fail("Deadlock was not found. Set SPLIT_DEADLOCK_ROOT to the installed Deadlock directory.");
}

async function validateStockVersion(deadlockRoot) {
  const steamInf = await readFile(join(deadlockRoot, "game", "citadel", "steam.inf"), "utf8");
  const value = (key) => steamInf.match(new RegExp(`^${key}=(.+)$`, "m"))?.[1].trim();
  const clientVersion = value("ClientVersion");
  const sourceRevision = value("SourceRevision");
  if (clientVersion !== stockClientVersion || sourceRevision !== stockSourceRevision) {
    fail(
      "The vendored stock HUD does not match the installed Deadlock build. " +
        `Expected client ${stockClientVersion} / source ${stockSourceRevision}; ` +
        `installed client ${clientVersion ?? "unknown"} / source ${sourceRevision ?? "unknown"}. ` +
        "Refresh panorama-addon/stock from the matching current GameTracking resource before building.",
    );
  }
}

function runTool(executable, args, label) {
  console.log(`\n[${label}] ${executable}`);
  const result = spawnSync(executable, args, {
    cwd: repoRoot,
    encoding: "utf8",
    windowsHide: true,
    maxBuffer: 32 * 1024 * 1024,
  });
  if (result.stdout?.trim()) console.log(result.stdout.trim());
  if (result.stderr?.trim()) console.error(result.stderr.trim());
  if (result.error) fail(`${label} could not start: ${result.error.message}`);
  if (result.status !== 0) fail(`${label} failed with exit code ${result.status}`);
}

function readIncludes(fragment, section) {
  const block = fragment.match(new RegExp(`<${section}>([\\s\\S]*?)</${section}>`))?.[1];
  if (!block) fail(`Missing <${section}> block in ${integrationLayout}.`);
  return [...block.matchAll(/<include\s+src="([^"]+)"\s*\/>/g)].map((match) => match[1]);
}

function splitIncludes(fragment) {
  const styles = readIncludes(fragment, "styles");
  const scripts = readIncludes(fragment, "scripts");
  if (JSON.stringify(styles) !== JSON.stringify(expectedStyleIncludes)) {
    fail(`Unexpected SPLIT style includes: ${styles.join(", ")}`);
  }
  if (JSON.stringify(scripts) !== JSON.stringify(expectedScriptIncludes)) {
    fail(`Unexpected SPLIT script includes or order: ${scripts.join(", ")}`);
  }
  return {
    style: `\t\t<include src="${styles[0]}" />`,
    scripts: [
      "\t<scripts>",
      ...scripts.map((source) => `\t\t<include src="${source}" />`),
      "\t</scripts>",
    ].join("\n"),
  };
}

function injectSplitIncludes(stock, includes) {
  const styleAnchor = '\t\t<include src="s2r://panorama/styles/ability_property_icons.vcss" />\n\t</styles>';
  const scriptAnchor = "\t<snippets>";
  if (stock.split(styleAnchor).length !== 2 || stock.split(scriptAnchor).length !== 2) {
    fail("The stock HUD layout no longer has the expected unique injection anchors.");
  }
  if (/split_(?:poc|quick_access)/i.test(stock)) {
    fail("The pristine stock HUD layout already contains a SPLIT/POC include.");
  }
  return stock
    .replace(styleAnchor, `${styleAnchor.slice(0, -"\t</styles>".length)}${includes.style}\n\t</styles>`)
    .replace(scriptAnchor, `${includes.scripts}\n${scriptAnchor}`);
}

async function listFiles(root) {
  const files = [];
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const fullPath = join(directory, entry.name);
      if (entry.isDirectory()) await visit(fullPath);
      else if (entry.isFile()) files.push(relative(root, fullPath).replaceAll("\\", "/"));
    }
  }
  await visit(root);
  return files.sort();
}

function assertExpectedFiles(actual, label) {
  if (JSON.stringify(actual) !== JSON.stringify(expectedFiles)) {
    fail(`${label} differs from the expected payload.\nExpected: ${expectedFiles.join(", ")}\nActual: ${actual.join(", ")}`);
  }
  if (actual.some((path) => /(?:split_poc|\.png$)/i.test(path))) {
    fail(`${label} contains a forbidden POC or PNG entry.`);
  }
}

function readCString(buffer, cursor, end) {
  const zero = buffer.indexOf(0, cursor);
  if (zero < cursor || zero >= end) fail("Malformed VPK directory tree.");
  return [buffer.toString("utf8", cursor, zero), zero + 1];
}

function listVpkEntries(buffer) {
  if (buffer.length < 12 || buffer.readUInt32LE(0) !== 0x55aa1234) {
    fail("Packed file does not have a supported Valve VPK header.");
  }
  const version = buffer.readUInt32LE(4);
  if (version !== 1 && version !== 2) fail(`Unsupported VPK version ${version}.`);
  const treeSize = buffer.readUInt32LE(8);
  const treeStart = version === 1 ? 12 : 28;
  const treeEnd = treeStart + treeSize;
  if (treeEnd > buffer.length) fail("VPK directory tree extends past the file.");

  let cursor = treeStart;
  const entries = [];
  while (true) {
    let extension;
    [extension, cursor] = readCString(buffer, cursor, treeEnd);
    if (!extension) break;
    while (true) {
      let directory;
      [directory, cursor] = readCString(buffer, cursor, treeEnd);
      if (!directory) break;
      while (true) {
        let filename;
        [filename, cursor] = readCString(buffer, cursor, treeEnd);
        if (!filename) break;
        if (cursor + 18 > treeEnd) fail("Truncated VPK directory entry.");
        const preloadBytes = buffer.readUInt16LE(cursor + 4);
        const terminator = buffer.readUInt16LE(cursor + 16);
        if (terminator !== 0xffff) fail("Invalid VPK directory entry terminator.");
        cursor += 18 + preloadBytes;
        const prefix = directory === " " ? "" : `${directory}/`;
        entries.push(`${prefix}${filename}.${extension}`.replaceAll("\\", "/"));
      }
    }
  }
  return entries.sort();
}

async function main() {
  const deadlockRoot = await findDeadlockRoot();
  await validateStockVersion(deadlockRoot);
  const csdkRoot = findCsdkRoot();
  const compiler = join(csdkRoot, "game", "bin_cs2", "win64", "resourcecompiler.exe");
  const packer = join(csdkRoot, "game", "bin", "win64", "CSDKCfgVPK.exe");
  const gameRoot = join(csdkRoot, "game", "citadel");
  const stagingContent = join(csdkRoot, "content", "citadel_addons", stagingName);
  const stagingGame = join(csdkRoot, "game", "citadel_addons", stagingName);

  assertChild(join(csdkRoot, "content", "citadel_addons"), stagingContent, "CSDK content staging");
  assertChild(join(csdkRoot, "game", "citadel_addons"), stagingGame, "CSDK game staging");
  await cleanDirectory(addonRoot, buildRoot, "local build directory");
  await mkdir(dirname(outputVpk), { recursive: true });
  await rm(outputVpk, { force: true });
  await cleanDirectory(join(csdkRoot, "content", "citadel_addons"), stagingContent, "CSDK content staging");
  await cleanDirectory(join(csdkRoot, "game", "citadel_addons"), stagingGame, "CSDK game staging");

  try {
    const pristineStock = await readFile(stockLayout, "utf8");
    const hash = createHash("sha256").update(pristineStock).digest("hex");
    if (hash !== stockSha256) {
      fail(`Stock HUD checksum changed: expected ${stockSha256}, got ${hash}.`);
    }
    const includes = splitIncludes(await readFile(integrationLayout, "utf8"));
    const splitLayout = injectSplitIncludes(pristineStock, includes);
    if (/split_poc/i.test(splitLayout)) fail("Generated layout contains an old POC include.");

    const stagedLayout = join(
      stagingContent,
      "panorama",
      "layout",
      "citadel_hud_active_player_stats.xml",
    );
    await mkdir(dirname(stagedLayout), { recursive: true });
    await writeFile(stagedLayout, splitLayout, "utf8");

    for (const source of sourceFiles) {
      const from = join(sourceRoot, source);
      const to = join(stagingContent, "panorama", source);
      await mkdir(dirname(to), { recursive: true });
      await copyFile(from, to);
    }

    const compileInputs = [stagedLayout, ...sourceFiles.map((source) => join(stagingContent, "panorama", source))];
    const compileArgs = compileInputs.flatMap((input) => ["-i", input]);
    compileArgs.push("-nop4", "-f", "-game", gameRoot);
    runTool(compiler, compileArgs, "resourcecompiler");

    for (const expected of expectedFiles) {
      const compiled = join(stagingGame, ...expected.split("/"));
      if (!existsSync(compiled) || !(await stat(compiled)).isFile()) {
        fail(`Missing compiled resource: ${compiled}`);
      }
      const destination = join(payloadRoot, ...expected.split("/"));
      await mkdir(dirname(destination), { recursive: true });
      await copyFile(compiled, destination);
    }
    assertExpectedFiles(await listFiles(payloadRoot), "Compiled payload");

    runTool(packer, [payloadRoot, outputVpk], "CSDKCfgVPK");
    if (!existsSync(outputVpk) || (await stat(outputVpk)).size === 0) {
      fail(`VPK was not produced at ${outputVpk}`);
    }
    const packedEntries = listVpkEntries(await readFile(outputVpk));
    assertExpectedFiles(packedEntries, "VPK contents");

    console.log("\nSPLIT Panorama addon build succeeded.");
    console.log(`Deadlock: ${deadlockRoot}`);
    console.log(`CSDK: ${csdkRoot}`);
    console.log(`VPK:  ${outputVpk}`);
    console.log("Contents:");
    for (const entry of packedEntries) console.log(`  ${entry}`);
  } finally {
    await rm(stagingContent, { recursive: true, force: true });
    await rm(stagingGame, { recursive: true, force: true });
  }
}

main().catch((error) => {
  console.error(`\nPanorama addon build failed: ${error.message}`);
  process.exitCode = 1;
});
