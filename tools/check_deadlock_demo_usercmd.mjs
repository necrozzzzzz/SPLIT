// Read-only probe: parse only user-command messages, without entity state or string tables.
// Usage: node tools/check_deadlock_demo_usercmd.mjs "C:\\path\\to\\split_test.dem"
import { createReadStream, existsSync } from "node:fs";
import { resolve } from "node:path";
import { Parser, ParserConfiguration, InterceptorStage, MessagePacketType } from "deadem";

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

// Deadem's default configuration builds full demo state, including string
// tables and entities. Some local Deadlock demos contain records that the
// current release cannot decode. We need neither of those for usercmd.
const parser = new Parser(new ParserConfiguration({
  messagePacketTypes: [MessagePacketType.SVC_USER_COMMANDS],
}));

let messagePackets = 0;
let userCmdPackets = 0;
let userCmdEvents = 0;
const perSlot = new Map();
const examples = [];
const errors = [];

parser.registerPreInterceptor(InterceptorStage.MESSAGE_PACKET, (_demoPacket, packet) => {
  messagePackets++;
  if (packet.type === MessagePacketType.SVC_USER_COMMANDS) userCmdPackets++;
});

parser.registerPostInterceptor(InterceptorStage.USER_COMMAND, (demoPacket, _packet, events) => {
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

console.log(JSON.stringify({
  file: source,
  messagePackets,
  userCmdPackets,
  userCmdEvents,
  slots: Object.fromEntries([...perSlot].sort((a,b) => Number(a[0])-Number(b[0]))),
  examples,
  errors,
}, (_key, value) => typeof value === "bigint" ? value.toString() : value, 2));
if (errors.length) process.exitCode = 1;
