import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const VAULT = "/mock/vault";
const WELCOME = `${VAULT}/Welcome.md`;

/** Opens Documents with the mock vault active and one note open. */
async function openWelcomeNote(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.removeItem("buzz.documents.vaultPath.v1");
  });
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("open-documents-view").click();
  await page.getByTestId("documents-choose-vault").click();
  await expect(page.getByTestId("documents-tree")).toBeVisible();
  await page.getByTestId("documents-file-Welcome.md").click();
  await expect(page.getByTestId("documents-tab-Welcome")).toBeVisible();
}

/**
 * The contenteditable ProseMirror surface.
 *
 * `EditorContent` renders a wrapper div; clicking that does not focus the
 * editable node, so keystrokes would go nowhere.
 */
function editorSurface(page: Page) {
  return page.locator('[data-testid="documents-live-editor"] .ProseMirror');
}

/** The mock vault's current contents for `path`. */
function readMockFile(page: Page, path: string) {
  return page.evaluate(
    (target) =>
      window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("read_vault_file", {
        path: target,
      }) as unknown as string,
    path,
  );
}

test.describe("Documents editing and autosave", () => {
  test("opens a clean note in live preview with no dirty marker", async ({
    page,
  }) => {
    await openWelcomeNote(page);

    await expect(page.getByTestId("documents-live-editor")).toBeVisible();
    // Opening a file must never mark it dirty — that is what would cause a
    // silent rewrite of a note the user only looked at.
    await expect(page.getByTestId("documents-tab-dirty-Welcome")).toHaveCount(
      0,
    );
  });

  test("typing marks the tab dirty and autosaves the edit", async ({
    page,
  }) => {
    await openWelcomeNote(page);

    const editor = editorSurface(page);
    await editor.click();
    await page.keyboard.press("End");
    await page.keyboard.type(" edited");

    await expect(page.getByTestId("documents-tab-dirty-Welcome")).toBeVisible();

    // Autosave is debounced; the dirty marker clearing is the signal it ran.
    await expect(page.getByTestId("documents-tab-dirty-Welcome")).toHaveCount(
      0,
      { timeout: 10_000 },
    );

    const saved = await readMockFile(page, WELCOME);
    expect(saved).toContain("edited");
  });

  test("Cmd+S saves immediately without waiting for the debounce", async ({
    page,
  }) => {
    await openWelcomeNote(page);

    const editor = editorSurface(page);
    await editor.click();
    await page.keyboard.press("End");
    await page.keyboard.type(" quicksave");
    await expect(page.getByTestId("documents-tab-dirty-Welcome")).toBeVisible();

    await page.keyboard.press("ControlOrMeta+s");
    await expect(page.getByTestId("documents-tab-dirty-Welcome")).toHaveCount(
      0,
      { timeout: 3_000 },
    );
    expect(await readMockFile(page, WELCOME)).toContain("quicksave");
  });

  test("switching to source mode shows the raw markdown", async ({ page }) => {
    await openWelcomeNote(page);

    await page.getByTestId("documents-toggle-view-mode").click();
    const source = page.getByTestId("documents-source-editor");
    await expect(source).toBeVisible();
    await expect(source).toHaveValue(/# Welcome/);
  });
});

test.describe("Documents round-trip guard", () => {
  test("a note containing a table opens in source mode with a warning", async ({
    page,
  }) => {
    // Raw HTML is escaped by the serializer (`<div>` becomes `&lt;div&gt;`), so
    // live-editing would corrupt it. Tables used to be the fixture here, but
    // they round-trip now that the table extensions are installed.
    await page.addInitScript(() => {
      window.localStorage.removeItem("buzz.documents.vaultPath.v1");
    });
    await installMockBridge(page);
    await page.goto("/");

    // Seed before activating the vault so the first tree read already has it.
    // The bridge assigns its helpers during app bootstrap, which can land after
    // `goto` resolves — waiting makes the seed deterministic instead of a
    // silently-skipped optional call.
    await page.waitForFunction(
      () => typeof window.__BUZZ_E2E_SEED_MOCK_VAULT_FILE__ === "function",
    );
    await page.evaluate(() => {
      const seed = window.__BUZZ_E2E_SEED_MOCK_VAULT_FILE__;
      if (!seed) throw new Error("mock vault seed helper is unavailable");
      seed(
        "/mock/vault/Raw note.md",
        '# Raw note\n\n<div align="center">centered</div>\n',
      );
    });

    await page.getByTestId("open-documents-view").click();
    await page.getByTestId("documents-choose-vault").click();
    await expect(page.getByTestId("documents-tree")).toBeVisible();
    await page.getByTestId("documents-file-Raw note.md").click();

    await expect(page.getByTestId("documents-round-trip-banner")).toBeVisible();
    await expect(page.getByTestId("documents-source-editor")).toBeVisible();
    await expect(page.getByTestId("documents-live-editor")).toHaveCount(0);
  });
});

test.describe("Documents external-change reconciliation", () => {
  test("a clean tab silently reloads when the file changes on disk", async ({
    page,
  }) => {
    await openWelcomeNote(page);

    await page.evaluate(() => {
      window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("write_vault_file", {
        content: "# Welcome\n\nChanged by another app.\n",
        path: "/mock/vault/Welcome.md",
      });
      window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("vault-file-modified", [
        { modified_ms: 999_999, path: "/mock/vault/Welcome.md" },
      ]);
    });

    await expect(editorSurface(page)).toContainText("Changed by another app.");
    await expect(
      page.getByTestId("documents-external-change-banner"),
    ).toHaveCount(0);
  });

  test("a dirty tab keeps the user's buffer and offers a choice", async ({
    page,
  }) => {
    await openWelcomeNote(page);

    const editor = editorSurface(page);
    await editor.click();
    await page.keyboard.press("End");
    await page.keyboard.type(" mine");
    await expect(page.getByTestId("documents-tab-dirty-Welcome")).toBeVisible();

    // The file must genuinely change on disk. Emitting the event alone proves
    // nothing: a watcher event whose bytes match what we already have is not a
    // conflict, and is deliberately ignored.
    await page.evaluate(() => {
      window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("write_vault_file", {
        content: "# Welcome\n\nChanged by another app.\n",
        path: "/mock/vault/Welcome.md",
      });
      window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("vault-file-modified", [
        { modified_ms: 999_999, path: "/mock/vault/Welcome.md" },
      ]);
    });

    // The unsaved edit must survive, and the pending autosave must be cancelled
    // rather than racing the external change.
    await expect(
      page.getByTestId("documents-external-change-banner"),
    ).toBeVisible();
    await expect(editor).toContainText("mine");

    await page.getByTestId("documents-external-keep").click();
    await expect(
      page.getByTestId("documents-external-change-banner"),
    ).toHaveCount(0);
    await expect(editor).toContainText("mine");
  });

  test("our own save echoing back is ignored", async ({ page }) => {
    await openWelcomeNote(page);

    const editor = editorSurface(page);
    await editor.click();
    await page.keyboard.press("End");
    await page.keyboard.type(" echoed");
    await page.keyboard.press("ControlOrMeta+s");
    await expect(page.getByTestId("documents-tab-dirty-Welcome")).toHaveCount(
      0,
      { timeout: 3_000 },
    );

    // Replay the watcher event the save itself would have produced, with the
    // mtime the app recorded. It must be recognised as an echo.
    await page.evaluate(() => {
      const mtime = window.__BUZZ_E2E_GET_MOCK_VAULT_MTIME__?.(
        "/mock/vault/Welcome.md",
      );
      window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("vault-file-modified", [
        { modified_ms: mtime, path: "/mock/vault/Welcome.md" },
      ]);
    });

    await expect(
      page.getByTestId("documents-external-change-banner"),
    ).toHaveCount(0);
    await expect(editor).toContainText("echoed");
  });

  test("a save whose watcher event arrives before its own response is not a conflict", async ({
    page,
  }) => {
    await openWelcomeNote(page);

    const editor = editorSurface(page);
    await editor.click();
    await page.keyboard.press("End");
    await page.keyboard.type(" first");

    // Let the debounced autosave land.
    await expect(page.getByTestId("documents-tab-dirty-Welcome")).toHaveCount(
      0,
      { timeout: 5_000 },
    );

    // Keep typing, so the tab is dirty again — the state the user is in while
    // working through a note.
    await page.keyboard.type(" second");
    await expect(page.getByTestId("documents-tab-dirty-Welcome")).toBeVisible();

    // The watcher event for the save above, carrying an mtime the app never
    // recorded. In the real app this happens because `write_vault_file`
    // replaces the file with a rename and the watcher fires on it before the
    // command's response crosses back — so mtime-based suppression cannot see
    // it. The bytes on disk are still ours, so this is not a conflict.
    await page.evaluate(() => {
      window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("vault-file-modified", [
        { modified_ms: 987_654, path: "/mock/vault/Welcome.md" },
      ]);
    });

    await expect(
      page.getByTestId("documents-external-change-banner"),
    ).toHaveCount(0);
    await expect(editor).toContainText("second");
  });

  test("a rewrite with identical content is not a conflict", async ({
    page,
  }) => {
    await openWelcomeNote(page);

    const editor = editorSurface(page);
    await editor.click();
    await page.keyboard.press("End");
    await page.keyboard.type(" mine");
    await expect(page.getByTestId("documents-tab-dirty-Welcome")).toBeVisible();

    // Sync clients and formatters rewrite files byte-for-byte all the time.
    // Nothing changed, so there is nothing to reconcile.
    await page.evaluate(() => {
      window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("vault-file-modified", [
        { modified_ms: 555_555, path: "/mock/vault/Welcome.md" },
      ]);
    });

    await expect(
      page.getByTestId("documents-external-change-banner"),
    ).toHaveCount(0);
    await expect(editor).toContainText("mine");
  });
});

test.describe("Documents tree mutations", () => {
  test("creates a note from the context menu and opens it", async ({
    page,
  }) => {
    await openWelcomeNote(page);

    await page.getByTestId("documents-file-Welcome.md").click({
      button: "right",
    });
    await page.getByTestId("documents-menu-new-note").click();

    const input = page.getByTestId("documents-name-input");
    await expect(input).toBeVisible();
    await input.fill("Fresh note");
    await page.getByTestId("documents-name-submit").click();

    // The new note opens ready to type in, and appears in the tree.
    await expect(page.getByTestId("documents-tab-Fresh note")).toBeVisible();
    await expect(
      page.getByTestId("documents-file-Fresh note.md"),
    ).toBeVisible();
  });

  test("rejects a name containing a slash before touching the filesystem", async ({
    page,
  }) => {
    await openWelcomeNote(page);

    await page.getByTestId("documents-file-Welcome.md").click({
      button: "right",
    });
    await page.getByTestId("documents-menu-new-note").click();
    await page.getByTestId("documents-name-input").fill("Notes/nested");

    await expect(page.getByTestId("documents-name-submit")).toBeDisabled();
  });

  test("deletes a note after confirmation and closes its tab", async ({
    page,
  }) => {
    await openWelcomeNote(page);
    await expect(page.getByTestId("documents-tab-Welcome")).toBeVisible();

    await page.getByTestId("documents-file-Welcome.md").click({
      button: "right",
    });
    await page.getByTestId("documents-menu-delete").click();
    await expect(page.getByTestId("documents-delete-dialog")).toBeVisible();
    await page.getByTestId("documents-delete-confirm").click();

    // The buffer must not linger for a file that no longer exists.
    await expect(page.getByTestId("documents-tab-Welcome")).toHaveCount(0);
    await expect(page.getByTestId("documents-file-Welcome.md")).toHaveCount(0);
  });
});

test.describe("Documents always-live-preview setting", () => {
  test("a lossy note opens in live preview when the setting is on", async ({
    page,
  }) => {
    // The guard still classifies the file — the setting changes which mode it
    // opens in, and suppresses the now-redundant warning.
    await page.addInitScript(() => {
      window.localStorage.removeItem("buzz.documents.vaultPath.v1");
      window.localStorage.setItem("buzz.documents.alwaysLivePreview.v1", "1");
    });
    await installMockBridge(page);
    await page.goto("/");
    await page.waitForFunction(
      () => typeof window.__BUZZ_E2E_SEED_MOCK_VAULT_FILE__ === "function",
    );
    await page.evaluate(() => {
      const seed = window.__BUZZ_E2E_SEED_MOCK_VAULT_FILE__;
      if (!seed) throw new Error("mock vault seed helper is unavailable");
      seed(
        "/mock/vault/Raw note.md",
        '# Raw note\n\n<div align="center">centered</div>\n',
      );
    });

    await page.getByTestId("open-documents-view").click();
    await page.getByTestId("documents-choose-vault").click();
    await expect(page.getByTestId("documents-tree")).toBeVisible();
    await page.getByTestId("documents-file-Raw note.md").click();

    await expect(page.getByTestId("documents-live-editor")).toBeVisible();
    await expect(page.getByTestId("documents-source-editor")).toHaveCount(0);
    // With the setting on the user has already accepted the reformatting, so
    // repeating the warning on every note is noise.
    await expect(page.getByTestId("documents-round-trip-banner")).toHaveCount(
      0,
    );
  });

  test("only one live-preview control is offered", async ({ page }) => {
    // Regression: the notice used to carry its own "use live preview anyway"
    // button alongside the header toggle, reading as two competing controls.
    await openWelcomeNote(page);
    await expect(page.getByTestId("documents-toggle-view-mode")).toHaveCount(1);
    await expect(page.getByTestId("documents-round-trip-switch")).toHaveCount(
      0,
    );
  });
});
