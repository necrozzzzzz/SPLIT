import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const context = vm.createContext({
  $: {
    SplitPanorama: {
      modules: { utils: {} },
      testHooks: {},
    },
    Warning() {},
  },
});

for (const file of [
  "panorama-addon/panorama/scripts/split_state.js",
  "panorama-addon/panorama/scripts/split_bridge.js",
]) {
  vm.runInContext(readFileSync(file, "utf8"), context, { filename: file });
}

const shared = context.$.SplitPanorama;
const protocol = shared.testHooks.bridgeProtocol;
const fastBytes = [1, 0, 0, 0x06, 2, 0x81, 7, 0, 0];
const fast = protocol.decodeFastState(fastBytes, 65535);

assert.equal(fast.sequence, 65536, "the 16-bit FAST sequence unwraps across rollover");
assert.equal(fast.visibility, "interactive");
assert.equal(fast.canUndo, true);
assert.equal(fast.canRedo, false);
assert.equal(fast.populatedMask, 0x81);
assert.equal(fast.metadataRevision, 7);

function metadataBytes(revision, preset, item, value) {
  const encoded = Buffer.from(value, "utf8");
  return [
    1,
    revision & 0xff,
    (revision >> 8) & 0xff,
    preset,
    item,
    encoded.length & 0xff,
    (encoded.length >> 8) & 0xff,
    ...encoded,
  ];
}

const presetItem = protocol.decodeMetadataItem(metadataBytes(7, 2, 0, "Routes α"));
const emptyItem = protocol.decodeMetadataItem(metadataBytes(7, 2, 2, ""));
assert.equal(presetItem.value, "Routes α");
assert.equal(emptyItem.value, "");

let metadataCache = protocol.createMetadataCache(fast);
assert.equal(protocol.metadataMatches(metadataCache, fast), true,
  "an unchanged revision reuses the partial metadata cache");
assert.equal(protocol.metadataMatches(metadataCache, { ...fast, metadataRevision: 8 }), false,
  "a changed revision invalidates metadata");

assert.equal(protocol.storeMetadataItem(metadataCache, presetItem), true);
assert.equal(protocol.storeMetadataItem(
  metadataCache,
  protocol.decodeMetadataItem(metadataBytes(7, 2, 1, "One")),
), true);
assert.equal(protocol.nextMissingMetadataItem(metadataCache), 2,
  "a failed third item resumes without restarting completed items");
assert.equal(metadataCache.items[0], "Routes α");
assert.equal(metadataCache.items[1], "One");

const prior = {
  sequence: 65535,
  activePreset: 1,
  activePresetName: "Old preset",
  slots: [],
};
const immediate = protocol.composeFastState(fast, prior, metadataCache);
assert.equal(immediate.visibility, "interactive");
assert.equal(immediate.slots[0].populated, true);
assert.equal(immediate.slots[7].populated, true);
assert.equal(immediate.slots[1].populated, false);
assert.equal(immediate.activePresetName, "Routes α");
assert.equal(immediate.slots[0].name, "One");
assert.equal(immediate.slots[1].name, "Slot 2",
  "FAST state is usable with partial metadata");

const state = shared.modules.createState();
const updates = [];
state.subscribe((value) => updates.push({
  name: value.activePresetName,
  populated: value.slots.length ? value.slots[0].populated : false,
}));
assert.equal(state.apply(immediate), true);
assert.equal(updates.at(-1).name, "Routes α");
assert.equal(updates.at(-1).populated, true);
const updatesBeforeItem = updates.length;
assert.equal(state.mergeMetadataItem(emptyItem), true);
assert.equal(updates.length, updatesBeforeItem + 1,
  "each metadata item publishes a partial state update");
assert.equal(state.get().slots[1].name, "");

for (let item = 2; item < 9; item += 1) {
  const value = item === 2 ? "" : `Item ${item}`;
  assert.equal(protocol.storeMetadataItem(
    metadataCache,
    protocol.decodeMetadataItem(metadataBytes(7, 2, item, value)),
  ), true);
}
assert.equal(protocol.nextMissingMetadataItem(metadataCache), -1,
  "all nine received items mark metadata complete");

const changedFast = { ...fast, metadataRevision: 8 };
metadataCache = protocol.createMetadataCache(changedFast);
assert.equal(protocol.nextMissingMetadataItem(metadataCache), 0,
  "a new revision starts with an empty item cache");

console.log("Panorama FAST/METADATA transport tests passed");
