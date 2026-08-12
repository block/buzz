import assert from "node:assert/strict";
import test from "node:test";

import {
  filesystemIsolationIsAvailable,
  filesystemIsolationProfilesEqual,
  isolationReadOnlyRootsAreAbsolute,
  parseIsolationReadOnlyRoots,
  resolveFilesystemIsolationUpdate,
} from "./filesystemIsolation.ts";

test("read-only root parser trims blanks and removes duplicates", () => {
  assert.deepEqual(
    parseIsolationReadOnlyRoots(
      " /opt/runtime \n\n/usr/local/share\n/opt/runtime\n",
    ),
    ["/opt/runtime", "/usr/local/share"],
  );
});

test("read-only roots must all be absolute", () => {
  assert.equal(
    isolationReadOnlyRootsAreAbsolute(["/opt/runtime", "/usr/local/share"]),
    true,
  );
  assert.equal(
    isolationReadOnlyRootsAreAbsolute(["/opt/runtime", "relative/path"]),
    false,
  );
});

test("profile equality ignores duplicate and presentation order", () => {
  assert.equal(
    filesystemIsolationProfilesEqual(
      {
        mode: "ephemeral",
        readOnlyRoots: ["/opt/runtime", "/usr/local/share"],
      },
      {
        mode: "ephemeral",
        readOnlyRoots: ["/usr/local/share", "/opt/runtime", "/opt/runtime"],
      },
    ),
    true,
  );
  assert.equal(
    filesystemIsolationProfilesEqual(
      { mode: "ephemeral", readOnlyRoots: [] },
      null,
    ),
    false,
  );
});

test("unchanged disabled profile emits no update", () => {
  assert.equal(resolveFilesystemIsolationUpdate(false, "", null), undefined);
});

test("enabling with no extra roots sends an explicit empty profile", () => {
  assert.deepEqual(resolveFilesystemIsolationUpdate(true, "\n", null), {
    mode: "ephemeral",
    readOnlyRoots: [],
  });
});

test("disabling an existing profile sends explicit null removal", () => {
  assert.equal(
    resolveFilesystemIsolationUpdate(false, "", {
      mode: "ephemeral",
      readOnlyRoots: ["/opt/runtime"],
    }),
    null,
  );
});

test("unchanged enabled profile emits no update", () => {
  assert.equal(
    resolveFilesystemIsolationUpdate(true, "/usr/local/share\n/opt/runtime\n", {
      mode: "ephemeral",
      readOnlyRoots: ["/opt/runtime", "/usr/local/share"],
    }),
    undefined,
  );
});

test("availability is explicit for local macOS agents only", () => {
  assert.equal(filesystemIsolationIsAvailable("local", true), true);
  assert.equal(filesystemIsolationIsAvailable("local", false), false);
  assert.equal(filesystemIsolationIsAvailable("provider", true), false);
});
