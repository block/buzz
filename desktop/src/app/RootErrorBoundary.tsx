import { Component, type ReactNode } from "react";

import { i18n } from "@/i18n";

type RootErrorBoundaryProps = {
  children: ReactNode;
};

type RootErrorBoundaryState = {
  error: Error | null;
};

/**
 * Root-level render fence for the desktop app (block/buzz#5078).
 *
 * Any uncaught throw inside the React tree — in particular a WebKit
 * `SecurityError` from `localStorage.getItem` under a denied-storage origin,
 * before the `safeStorage` accessors have a chance to fence it — previously
 * propagated to the reconciler's error boundary (there isn't one) and left
 * the window blank. This boundary renders a degraded splash instead so the
 * user always sees something actionable, and the throw is logged at least
 * once for diagnosis.
 *
 * Its copy resolves in `render()`, not at module scope: a class component has
 * no hook to take, and the bootstrap boots i18n before it mounts the tree this
 * fence wraps (the language module fences denied storage on its own).
 */
export class RootErrorBoundary extends Component<
  RootErrorBoundaryProps,
  RootErrorBoundaryState
> {
  override state: RootErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: unknown): RootErrorBoundaryState {
    return { error: error instanceof Error ? error : new Error(String(error)) };
  }

  override componentDidCatch(error: unknown, info: React.ErrorInfo): void {
    console.error("[RootErrorBoundary] uncaught render error:", error, info);
  }

  override render(): ReactNode {
    const { error } = this.state;
    if (error) {
      return (
        <div className="flex h-screen w-screen flex-col items-center justify-center gap-3 bg-background px-6 text-foreground">
          <p className="text-base font-semibold">
            {i18n.t("shared.app-shell.error-title")}
          </p>
          <p className="max-w-md text-center text-sm text-muted-foreground">
            {i18n.t("shared.app-shell.error-body")}
          </p>
          <button
            type="button"
            className="rounded-md border border-border bg-secondary px-4 py-2 text-sm hover:bg-secondary/80"
            onClick={() => window.location.reload()}
          >
            {i18n.t("shared.app-shell.reload")}
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
