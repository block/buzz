import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const component = readFileSync(
  new URL("./OwnerAttestationSettingsCard.tsx", import.meta.url),
  "utf8",
);
const backend = readFileSync(
  new URL(
    "../../../../src-tauri/src/commands/owner_attestation.rs",
    import.meta.url,
  ),
  "utf8",
);

test("owner attestation signing crosses IPC only with a one-use preview id", () => {
  assert.match(component, /previewId:\s*string/);
  assert.doesNotMatch(component, /requestPath:\s*string/);
  assert.doesNotMatch(component, /requestSha256:\s*string/);

  const invocation = component.match(
    /invokeTauri<void>\("sign_owner_attestation_request",\s*\{([\s\S]*?)\}\);/,
  );
  assert.ok(invocation, "signing invocation is present");
  assert.match(invocation[1], /previewId:\s*preview\.previewId/);
  assert.doesNotMatch(invocation[1], /requestPath|requestSha256|ownerPubkey/);
});

test("the native command consumes the preview and owns final confirmation", () => {
  const signature = backend.match(
    /pub async fn sign_owner_attestation_request\(([\s\S]*?)\) -> Result<\(\), String>/,
  );
  assert.ok(signature, "native signing command is present");
  assert.match(signature[1], /preview_id:\s*String/);
  assert.doesNotMatch(
    signature[1],
    /request_path|expected_request_sha256|expected_owner_pubkey/,
  );
  assert.match(
    backend,
    /owner_attestation_previews[\s\S]*?\.take\(&preview_id\)\?/,
  );
  assert.match(backend, /\.blocking_show\(\)/);
  assert.match(backend, /MessageDialogButtons::OkCancelCustom/);
});

test("a signing attempt clears the consumed one-use preview", () => {
  const signHandler = component.match(
    /async function signRequest\(\) \{([\s\S]*?)\n {2}\}/,
  )?.[1];
  assert.ok(signHandler, "signOnce handler should exist");

  const clearIndex = signHandler.indexOf("setPreview(null)");
  const invokeIndex = signHandler.indexOf(
    'invokeTauri<void>("sign_owner_attestation_request"',
  );
  assert.ok(clearIndex >= 0, "signing must clear the one-use preview");
  assert.ok(invokeIndex >= 0, "signing command should still be invoked");
  assert.ok(
    clearIndex < invokeIndex,
    "preview must clear before the backend consumes its authorization",
  );
});
