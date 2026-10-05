import { spawnSync } from "node:child_process";

const [edition, command, ...args] = process.argv.slice(2);

if (!edition || !command) {
  console.error(
    "Usage: node scripts/run-edition.mjs <borderless|fullscreen> <command> [...args]",
  );
  process.exit(2);
}

if (edition !== "borderless" && edition !== "fullscreen") {
  console.error(`Unsupported SPLIT edition: ${edition}`);
  process.exit(2);
}

const npmCli = command === "npm" ? process.env.npm_execpath : undefined;
const executable = npmCli ? process.execPath : command;
const commandArgs = npmCli ? [npmCli, ...args] : args;

const result = spawnSync(executable, commandArgs, {
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

process.exit(result.status ?? 1);
