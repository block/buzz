import * as React from "react";

import type { Editor } from "@tiptap/react";

import { mountSpoilerParticleCanvas } from "@/shared/ui/SpoilerParticles";

type ComposerSpoilerMount = {
  canvas: HTMLCanvasElement;
  cleanup: () => void;
};

const SPOILER_SELECTOR = ".buzz-spoiler[data-spoiler]";

export function useComposerSpoilerParticles(
  editor: Editor | null,
  scrollRef: React.RefObject<HTMLElement | null>,
) {
  React.useEffect(() => {
    if (!editor) return;

    let cancelled = false;
    let setupFrame = 0;
    let teardown: (() => void) | undefined;

    const setup = () => {
      let editorRoot: HTMLElement;
      try {
        editorRoot = editor.view.dom as HTMLElement;
      } catch {
        if (!cancelled) setupFrame = window.requestAnimationFrame(setup);
        return;
      }

      const scrollRoot = scrollRef.current ?? editorRoot.parentElement;
      if (!scrollRoot) {
        if (!cancelled) setupFrame = window.requestAnimationFrame(setup);
        return;
      }

      teardown = mountComposerSpoilerParticles(editor, editorRoot, scrollRoot);
    };

    setup();

    return () => {
      cancelled = true;
      if (setupFrame) {
        window.cancelAnimationFrame(setupFrame);
      }
      teardown?.();
    };
  }, [editor, scrollRef]);
}

function mountComposerSpoilerParticles(
  editor: Editor,
  editorRoot: HTMLElement,
  scrollRoot: HTMLElement,
): () => void {
  const document = editorRoot.ownerDocument;
  const overlayRoot = document.createElement("div");
  overlayRoot.className = "buzz-spoiler-composer-particles";
  scrollRoot.appendChild(overlayRoot);

  const mounts = new Map<HTMLElement, ComposerSpoilerMount>();
  let syncFrame = 0;

  const syncCanvasGeometry = (
    spoiler: HTMLElement,
    canvas: HTMLCanvasElement,
  ) => {
    if (
      !spoiler.isConnected ||
      !scrollRoot.isConnected ||
      !editorRoot.contains(spoiler)
    ) {
      canvas.style.display = "none";
      return false;
    }

    const spoilerRect = spoiler.getBoundingClientRect();
    const scrollRect = scrollRoot.getBoundingClientRect();
    const fontSize = Number.parseFloat(getComputedStyle(spoiler).fontSize);
    const padding = Number.isFinite(fontSize) ? fontSize * 0.1 : 1.6;
    const width = spoilerRect.width + padding * 2;
    const height = spoilerRect.height + padding * 2;

    if (width <= 0 || height <= 0) {
      canvas.style.display = "none";
      return false;
    }

    canvas.style.display = "block";
    canvas.style.left = `${
      spoilerRect.left - scrollRect.left + scrollRoot.scrollLeft - padding
    }px`;
    canvas.style.top = `${
      spoilerRect.top - scrollRect.top + scrollRoot.scrollTop - padding
    }px`;
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    return true;
  };

  const mountSpoiler = (spoiler: HTMLElement) => {
    const canvas = document.createElement("canvas");
    canvas.className =
      "buzz-spoiler__particles buzz-spoiler__particles--composer";
    overlayRoot.appendChild(canvas);

    const cleanup = mountSpoilerParticleCanvas({
      canvas,
      content: spoiler,
      syncCanvas: () => syncCanvasGeometry(spoiler, canvas),
    });

    mounts.set(spoiler, { canvas, cleanup });
  };

  const syncSpoilers = () => {
    syncFrame = 0;
    const spoilers = new Set(
      Array.from(editorRoot.querySelectorAll<HTMLElement>(SPOILER_SELECTOR)),
    );

    for (const spoiler of spoilers) {
      if (!mounts.has(spoiler)) {
        mountSpoiler(spoiler);
      }
    }

    for (const [spoiler, mount] of mounts) {
      if (!spoilers.has(spoiler)) {
        mount.cleanup();
        mount.canvas.remove();
        mounts.delete(spoiler);
        continue;
      }

      syncCanvasGeometry(spoiler, mount.canvas);
    }
  };

  const scheduleSync = () => {
    if (syncFrame) return;
    syncFrame = window.requestAnimationFrame(syncSpoilers);
  };

  const mutationObserver = new MutationObserver(scheduleSync);
  mutationObserver.observe(editorRoot, {
    attributes: true,
    characterData: true,
    childList: true,
    subtree: true,
  });

  const handleTransaction = () => scheduleSync();
  editor.on("transaction", handleTransaction);
  scrollRoot.addEventListener("scroll", scheduleSync, { passive: true });
  window.addEventListener("resize", scheduleSync);
  window.addEventListener("scroll", scheduleSync, true);

  scheduleSync();

  return () => {
    if (syncFrame) {
      window.cancelAnimationFrame(syncFrame);
    }
    editor.off("transaction", handleTransaction);
    mutationObserver.disconnect();
    scrollRoot.removeEventListener("scroll", scheduleSync);
    window.removeEventListener("resize", scheduleSync);
    window.removeEventListener("scroll", scheduleSync, true);
    for (const mount of mounts.values()) {
      mount.cleanup();
      mount.canvas.remove();
    }
    overlayRoot.remove();
  };
}
