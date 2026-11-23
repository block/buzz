// Replays the recorded huddle sequences through desktop's own tracker and
// checks it agrees with the rosters the CLI tests expect.
//   node --import ./desktop/test-loader.mjs --experimental-strip-types \
//     crates/buzz-cli/tests/huddle_interop.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const { HuddlePresenceTracker, applyHuddleLifecycleHistory } = await import(
  new URL("../../../desktop/src/features/huddle/lib/huddlePresence.ts", import.meta.url)
);
const sequences = JSON.parse(
  readFileSync(new URL("./fixtures/huddle_sequences.json", import.meta.url), "utf8"),
);

for (const sequence of sequences) {
  const tracker = new HuddlePresenceTracker(sequence.relay_self);
  applyHuddleLifecycleHistory(tracker, sequence.events);
  const roster = [...tracker.snapshot()].sort();
  const ended = Boolean(tracker.sessions.get(sequence.huddle)?.endState);
  assert.deepEqual(roster, sequence.expected_participants, `${sequence.name}: roster`);
  assert.equal(ended, sequence.expected_ended, `${sequence.name}: ended`);
  console.log(`ok ${sequence.name}`);
}
console.log("INTEROP OK");
