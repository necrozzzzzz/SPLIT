// Probe a Deadlock .dem for actual SVC_USER_COMMANDS (read-only).
// Usage: node tools/check_deadlock_demo_usercmd.mjs "C:\\path\\to\\split_test.dem"
import { createReadStream, existsSync } from "node:fs";
import { resolve } from "node:path";
import { Parser, InterceptorStage, MessagePacketType } from "deadem";

const filename = process.argv[2];
if (!filename) {
  console.error('Usage: node tools/check_deadlock_demo_usercmd.mjs "path/to/demo.dem"');
  process.exit(2);
}
const source = resolve(filename);
if (!existsSync(source)) {
  console.error("Demo not found:", source);
  process.exit(2);
}

// Some locally recorded Deadlock demos update unrecognized tables such as
// CombatLogNames without associated decoding instructions. Deadem 4.x throws
// while reading them. Skip only that known failure so the message stream can
// continue; this diagnostic does not need those string-table entries.
const { default: StringTableHandler } = await import(
  "../node_modules/@deademx/engine/src/handlers/StringTableHandler.js"
);
const originalHandleUpdate = StringTableHandler.prototype.handleUpdate;
let skippedUnknownTableUpdates = 0;
StringTableHandler.prototype.handleUpdate = function (...args) {
  try {
    return originalHandleUpdate.apply(this, args);
  } catch (error) {
    if (error instanceof TypeError &&
        String(error.message).includes("userDataFixedSize")) {
      skippedUnknownTableUpdates++;
      if (skippedUnknownTableUpdates <= 5) {
        console.warn("[SPLIT] Skipping unknown string-table update:", error.message);
      }
      return;
    }
    throw error;
  }
};

const parser = new Parser();
let userCmdPackets = 0;
let userCmdEvents = 0;
let messagePackets = 0;
const perSlot = new Map();
const examples = [];
const errors = [];

parser.registerPostInterceptor(InterceptorStage.MESSAGE_PACKET, (demoPacket, packet) => {
  messagePackets++;
  if (packet.type === MessagePacketType.SVC_USER_COMMANDS) {
    userCmdPackets++;
  }
});

parser.registerPostInterceptor(InterceptorStage.USER_COMMAND, (demoPacket, packet, events) => {
  userCmdEvents += events.length;
  for (const event of events) {
    const cmd = event.userCommand;
    const slot = cmd.slot;
    perSlot.set(slot, (perSlot.get(slot) ?? 0) + 1);
    if (examples.length < 12) {
      const base = cmd.state.base ?? {};
      const buttons = base.buttonsPb ?? {};
      examples.push({
        tick: demoPacket.tick,
        slot,
        commandNumber: base.commandNumber ?? null,
        clientTick: base.clientTick ?? null,
        buttonstate1: buttons.buttonstate1 ?? null,
        forwardmove: base.forwardmove ?? null,
        leftmove: base.leftmove ?? null,
        viewangles: base.viewangles ?? null,
      });
    }
  }
});

try {
  await parser.parse(createReadStream(source));
} catch (error) {
  errors.push(String(error?.stack ?? error));
} finally {
  try { await parser.dispose(); } catch {}
}

const json = JSON.stringify({
  file: source,
  messagePackets,
  skippedUnknownTableUpdates,
  userCmdPackets,
  userCmdEvents,
  slots: Object.fromEntries([...perSlot].sort((a,b) => Number(a[0])-Number(b[0]))),
  examples,
  errors,
}, (_key, value) => typeof value === "bigint" ? value.toString() : value, 2);
console.log(json);
if (errors.length) process.exitCode = 1;
