import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  Check,
  Cloud,
  FolderOpen,
  Link2,
  LoaderCircle,
  Network,
} from "lucide-react";
import * as React from "react";

import {
  useConnectLocalWorldAuthorityMutation,
  useRegisterHostedWorldAuthorityMutation,
  useTrustWorldOriginMutation,
  usePublishHostedWorldLiveViewShareMutation,
  useWorldAuthoritiesQuery,
  useWorldViewCatalogQuery,
} from "@/features/channels/hooks";
import { parsePublicWorldViewReference } from "@/features/channels/worldViewDescriptor";
import type {
  WorldViewAuthority,
  WorldViewBinding,
  WorldViewBindingScope,
  WorldViewBindingsDocument,
  WorldViewCatalog,
  WorldViewCatalogEntry,
  WorldViewReference,
} from "@/shared/api/worldViewTypes";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

const DEFAULT_SHIVAI_ORIGIN = "https://manifest.shivai.space";

type WorldViewBindingEditorState = {
  displayMode: WorldViewBinding["displayMode"];
  hostedCredential: string;
  isHostedConnectionOpen: boolean;
  label: string;
  publicReference: string;
  pendingPublicReference: {
    reference: WorldViewReference;
    selectedViewQualifiedName: string;
  } | null;
  publicReferenceError: string | null;
  reference: WorldViewReference | null;
  selectedViewQualifiedName: string;
  sourceError: string | null;
};

type WorldViewBindingEditorAction = {
  type: "patch";
  value: Partial<WorldViewBindingEditorState>;
};

function createWorldViewBindingEditorState(
  binding: WorldViewBinding | null,
): WorldViewBindingEditorState {
  return {
    displayMode: binding?.displayMode ?? "graph",
    hostedCredential: "",
    isHostedConnectionOpen: false,
    label: binding?.label ?? "",
    pendingPublicReference: null,
    publicReference: "",
    publicReferenceError: null,
    reference: binding?.reference ?? null,
    selectedViewQualifiedName: binding?.viewQualifiedName ?? "",
    sourceError: null,
  };
}

function worldViewBindingEditorReducer(
  state: WorldViewBindingEditorState,
  action: WorldViewBindingEditorAction,
): WorldViewBindingEditorState {
  return { ...state, ...action.value };
}

function worldViewReferenceKey(reference: WorldViewReference): string {
  switch (reference.kind) {
    case "local-world-mirror-latest":
      return `${reference.kind}:${reference.origin}:${reference.mirrorId}`;
    case "hosted-world-latest":
      return `${reference.kind}:${reference.origin}:${reference.hostedWorldId}`;
    case "hosted-world-view-export":
    case "hosted-world-live-view-share":
      return `${reference.kind}:${reference.origin}:${reference.shareToken}`;
  }
}

function worldViewReferencesEqual(
  left: WorldViewReference | null,
  right: WorldViewReference,
): boolean {
  return (
    left !== null &&
    worldViewReferenceKey(left) === worldViewReferenceKey(right)
  );
}

function publicWorldViewReference(
  authority: WorldViewAuthority,
): WorldViewReference {
  switch (authority.kind) {
    case "local-world-mirror-latest":
      return {
        kind: authority.kind,
        origin: authority.origin,
        mirrorId: authority.mirrorId,
      };
    case "hosted-world-latest":
      return {
        kind: authority.kind,
        origin: authority.origin,
        hostedWorldId: authority.hostedWorldId,
      };
  }
}

function localWorldName(sourceRoot: string): string {
  const segments = sourceRoot.replaceAll("\\", "/").split("/");
  return segments.at(-1) || "Local world";
}

function hostedCredentialOrigin(credential: string): string {
  try {
    return new URL(credential).origin;
  } catch {
    return DEFAULT_SHIVAI_ORIGIN;
  }
}

export type ChannelWorldViewBindingEditorProps = {
  binding: WorldViewBinding | null;
  bindings: readonly WorldViewBinding[];
  bindingScope: WorldViewBindingScope;
  isPublishing: boolean;
  onComplete: () => void;
  onPublish: (document: WorldViewBindingsDocument) => Promise<void>;
  publishError: Error | null;
};

export function ChannelWorldViewBindingEditor({
  binding,
  bindings,
  bindingScope,
  isPublishing,
  onComplete,
  onPublish,
  publishError,
}: ChannelWorldViewBindingEditorProps) {
  const authoritiesQuery = useWorldAuthoritiesQuery();
  const localAuthorityMutation = useConnectLocalWorldAuthorityMutation();
  const hostedAuthorityMutation = useRegisterHostedWorldAuthorityMutation();
  const liveViewShareMutation = usePublishHostedWorldLiveViewShareMutation();
  const trustOriginMutation = useTrustWorldOriginMutation();
  const [state, dispatch] = React.useReducer(
    worldViewBindingEditorReducer,
    createWorldViewBindingEditorState(binding),
  );
  const authorities = authoritiesQuery.data?.authorities ?? [];
  const trustedOrigins = authoritiesQuery.data?.trustedOrigins ?? [];
  const referenceIsTrusted =
    state.reference !== null && trustedOrigins.includes(state.reference.origin);
  const catalogQuery = useWorldViewCatalogQuery(
    referenceIsTrusted ? state.reference : null,
  );
  const selectedView = catalogQuery.data?.views.find(
    (view) => view.qualifiedName === state.selectedViewQualifiedName,
  );
  const trustReference =
    state.pendingPublicReference?.reference ??
    (state.reference && authoritiesQuery.data && !referenceIsTrusted
      ? state.reference
      : null);
  const isBusy =
    isPublishing ||
    localAuthorityMutation.isPending ||
    hostedAuthorityMutation.isPending ||
    liveViewShareMutation.isPending ||
    trustOriginMutation.isPending;

  React.useEffect(() => {
    const firstView = catalogQuery.data?.views[0];
    if (state.selectedViewQualifiedName || !firstView) return;
    dispatch({
      type: "patch",
      value: {
        selectedViewQualifiedName: firstView.qualifiedName,
      },
    });
  }, [catalogQuery.data, state.selectedViewQualifiedName]);

  function patch(value: Partial<WorldViewBindingEditorState>): void {
    dispatch({ type: "patch", value });
  }

  function selectReference(
    reference: WorldViewReference,
    selectedViewQualifiedName = "",
  ): void {
    patch({
      publicReferenceError: null,
      pendingPublicReference: null,
      reference,
      selectedViewQualifiedName,
      sourceError: null,
    });
  }

  function handleUsePublicReference(): void {
    const parsed = parsePublicWorldViewReference(state.publicReference);
    if (!parsed.ok) {
      patch({ publicReferenceError: parsed.error });
      return;
    }
    const selectedViewQualifiedName =
      parsed.value.selection?.viewQualifiedName ?? "";
    if (trustedOrigins.includes(parsed.value.reference.origin)) {
      selectReference(parsed.value.reference, selectedViewQualifiedName);
      return;
    }
    patch({
      pendingPublicReference: {
        reference: parsed.value.reference,
        selectedViewQualifiedName,
      },
      publicReferenceError: null,
    });
  }

  async function handleTrustOrigin(): Promise<void> {
    if (!trustReference) return;
    await trustOriginMutation.mutateAsync({ origin: trustReference.origin });
    if (state.pendingPublicReference) {
      selectReference(
        state.pendingPublicReference.reference,
        state.pendingPublicReference.selectedViewQualifiedName,
      );
    }
  }

  function handleCancelOriginTrust(): void {
    if (state.pendingPublicReference) {
      patch({ pendingPublicReference: null });
      return;
    }
    patch({
      reference: null,
      selectedViewQualifiedName: "",
    });
  }

  async function handleConnectLocalWorld(): Promise<void> {
    patch({ sourceError: null });
    try {
      const sourceRoot = await openDialog({
        directory: true,
        multiple: false,
        title: "Choose a published Shivai world",
      });
      if (typeof sourceRoot !== "string") return;
      const result = await localAuthorityMutation.mutateAsync({ sourceRoot });
      selectReference(result.worldRef);
    } catch (error) {
      patch({
        sourceError:
          error instanceof Error
            ? error.message
            : "Could not connect local world.",
      });
    }
  }

  async function handleConnectHostedWorld(): Promise<void> {
    const credential = state.hostedCredential.trim();
    if (!credential) return;
    patch({ sourceError: null });
    try {
      const result = await hostedAuthorityMutation.mutateAsync({
        credential,
        origin: hostedCredentialOrigin(credential),
      });
      patch({
        hostedCredential: "",
        isHostedConnectionOpen: false,
      });
      selectReference(result.worldRef);
    } catch {
      // The mutation owns the user-visible error state.
    }
  }

  async function handleSubmit(
    event: React.FormEvent<HTMLFormElement>,
  ): Promise<void> {
    event.preventDefault();
    if (!state.reference || !selectedView) return;

    const publicReference =
      state.reference.kind === "hosted-world-latest"
        ? await liveViewShareMutation
            .mutateAsync({
              reference: state.reference,
              viewQualifiedName: selectedView.qualifiedName,
            })
            .then(
              (share): WorldViewReference => ({
                kind: "hosted-world-live-view-share",
                origin: state.reference?.origin ?? DEFAULT_SHIVAI_ORIGIN,
                shareToken: share.shareToken,
              }),
            )
        : state.reference;
    const nextBinding: WorldViewBinding = {
      id: binding?.id ?? crypto.randomUUID(),
      ...(state.label.trim() ? { label: state.label.trim() } : {}),
      reference: publicReference,
      realmQualifiedName: selectedView.realm.qualifiedName,
      viewQualifiedName: selectedView.qualifiedName,
      displayMode: state.displayMode,
    };
    const replacesExactBinding = bindings.some(
      (candidate) => candidate.id === nextBinding.id,
    );
    const nextBindings = replacesExactBinding
      ? bindings.map((candidate) =>
          candidate.id === nextBinding.id ? nextBinding : candidate,
        )
      : [...bindings, nextBinding];
    await onPublish({
      version: 4,
      scope: bindingScope,
      bindings: nextBindings,
    });
    onComplete();
  }

  const mutationError =
    localAuthorityMutation.error instanceof Error
      ? localAuthorityMutation.error
      : hostedAuthorityMutation.error instanceof Error
        ? hostedAuthorityMutation.error
        : liveViewShareMutation.error instanceof Error
          ? liveViewShareMutation.error
          : trustOriginMutation.error instanceof Error
            ? trustOriginMutation.error
            : null;
  const queryError =
    authoritiesQuery.error instanceof Error
      ? authoritiesQuery.error
      : catalogQuery.error instanceof Error
        ? catalogQuery.error
        : null;
  const submitLabel = binding
    ? "Save view"
    : bindingScope.kind === "thread"
      ? "Share in this thread"
      : "Share in this channel";

  return (
    <form
      className="mx-auto grid w-full max-w-3xl gap-4 rounded-2xl border border-border/70 bg-background/90 p-4 shadow-xl shadow-black/10"
      onSubmit={(event) => void handleSubmit(event)}
    >
      <header className="flex items-start gap-3">
        <span className="grid h-10 w-10 shrink-0 place-items-center rounded-xl border border-primary/20 bg-primary/10 text-primary shadow-sm">
          <Network className="h-5 w-5" />
        </span>
        <div className="min-w-0">
          <h3 className="text-sm font-semibold text-foreground">
            {binding ? "Edit Shivai view" : "Add Shivai view"}
          </h3>
          <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
            Choose a world connected to this device, then select the view to
            share with this conversation.
          </p>
        </div>
      </header>

      <WorldSourcePicker
        actions={{
          onConnectHostedWorld: handleConnectHostedWorld,
          onConnectLocalWorld: handleConnectLocalWorld,
          onHostedCredentialChange: (hostedCredential) =>
            patch({ hostedCredential }),
          onPublicReferenceChange: (publicReference) =>
            patch({
              pendingPublicReference: null,
              publicReference,
              publicReferenceError: null,
            }),
          onCancelOriginTrust: handleCancelOriginTrust,
          onTrustOrigin: handleTrustOrigin,
          onSelectReference: selectReference,
          onToggleHostedConnection: () =>
            patch({
              isHostedConnectionOpen: !state.isHostedConnectionOpen,
              sourceError: null,
            }),
          onUsePublicReference: handleUsePublicReference,
        }}
        model={{
          authorities,
          hostedCredential: state.hostedCredential,
          isTrustingOrigin: trustOriginMutation.isPending,
          trustOrigin: trustReference?.origin ?? null,
          isBusy,
          isFetchingAuthorities: authoritiesQuery.isFetching,
          isHostedConnecting: hostedAuthorityMutation.isPending,
          isHostedConnectionOpen: state.isHostedConnectionOpen,
          publicReference: state.publicReference,
          publicReferenceError: state.publicReferenceError,
          reference: state.reference,
        }}
      />

      <WorldViewSettings
        actions={{
          onDisplayModeChange: (displayMode) => patch({ displayMode }),
          onLabelChange: (label) => patch({ label }),
          onViewChange: (selectedViewQualifiedName) =>
            patch({ selectedViewQualifiedName }),
        }}
        model={{
          catalog: catalogQuery.data,
          displayMode: state.displayMode,
          isCatalogFetching: catalogQuery.isFetching,
          label: state.label,
          referenceSelected: state.reference !== null,
          selectedView,
          selectedViewQualifiedName: state.selectedViewQualifiedName,
        }}
      />

      {state.sourceError ? (
        <p className="text-xs text-destructive">{state.sourceError}</p>
      ) : mutationError ? (
        <p className="text-xs text-destructive">{mutationError.message}</p>
      ) : queryError ? (
        <p className="text-xs text-destructive">{queryError.message}</p>
      ) : publishError ? (
        <p className="text-xs text-destructive">{publishError.message}</p>
      ) : null}

      <footer className="flex flex-col-reverse gap-2 border-t border-border/60 pt-3 sm:flex-row sm:items-center sm:justify-between">
        <p className="text-xs leading-relaxed text-muted-foreground">
          Private paths and edit credentials remain on this device.
        </p>
        <Button
          className="shrink-0"
          disabled={isBusy || !state.reference || !selectedView}
          size="sm"
          type="submit"
        >
          {isBusy ? "Sharing..." : submitLabel}
        </Button>
      </footer>
    </form>
  );
}

type WorldSourcePickerModel = {
  authorities: readonly WorldViewAuthority[];
  hostedCredential: string;
  isBusy: boolean;
  isFetchingAuthorities: boolean;
  isTrustingOrigin: boolean;
  isHostedConnecting: boolean;
  isHostedConnectionOpen: boolean;
  publicReference: string;
  publicReferenceError: string | null;
  reference: WorldViewReference | null;
  trustOrigin: string | null;
};

type WorldSourcePickerActions = {
  onConnectHostedWorld: () => Promise<void>;
  onCancelOriginTrust: () => void;
  onConnectLocalWorld: () => Promise<void>;
  onHostedCredentialChange: (value: string) => void;
  onPublicReferenceChange: (value: string) => void;
  onSelectReference: (reference: WorldViewReference) => void;
  onToggleHostedConnection: () => void;
  onUsePublicReference: () => void;
  onTrustOrigin: () => Promise<void>;
};

type WorldSourcePickerProps = {
  actions: WorldSourcePickerActions;
  model: WorldSourcePickerModel;
};

function WorldSourcePicker({ actions, model }: WorldSourcePickerProps) {
  const hostedCredentialInputId = React.useId();
  const publicReferenceInputId = React.useId();
  const selectedReferenceIsShared =
    model.reference?.kind === "hosted-world-view-export" ||
    model.reference?.kind === "hosted-world-live-view-share";

  return (
    <>
      <section className="grid gap-2">
        <div className="flex items-center justify-between gap-3">
          <h4 className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
            Connected worlds
          </h4>
          {model.isFetchingAuthorities ? (
            <LoaderCircle className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
          ) : null}
        </div>
        <div className="overflow-hidden rounded-xl border border-border/70 bg-muted/20">
          {model.authorities.length === 0 && !selectedReferenceIsShared ? (
            <p className="px-3 py-4 text-center text-xs text-muted-foreground">
              No worlds are connected to this device yet.
            </p>
          ) : (
            <div className="divide-y divide-border/60">
              {model.authorities.map((authority) => {
                const reference = publicWorldViewReference(authority);
                return (
                  <WorldAuthorityChoice
                    authority={authority}
                    isSelected={worldViewReferencesEqual(
                      model.reference,
                      reference,
                    )}
                    key={worldViewReferenceKey(reference)}
                    onSelect={() => actions.onSelectReference(reference)}
                  />
                );
              })}
              {selectedReferenceIsShared ? (
                <button
                  aria-pressed="true"
                  className="flex w-full items-center gap-3 bg-primary/5 px-3 py-2.5 text-left"
                  type="button"
                >
                  <span className="grid h-8 w-8 place-items-center rounded-lg bg-blue-9/10 text-blue-11">
                    <Link2 className="h-4 w-4" />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium text-foreground">
                      {model.reference?.kind === "hosted-world-live-view-share"
                        ? "Shared live read-only view"
                        : "Shared pinned read-only view"}
                    </span>
                    <span className="block truncate text-3xs text-muted-foreground">
                      {model.reference?.origin}
                    </span>
                  </span>
                  <Check className="h-4 w-4 text-primary" />
                </button>
              ) : null}
            </div>
          )}
        </div>
        <div className="grid gap-2 sm:grid-cols-2">
          <Button
            className="h-9 justify-start gap-2"
            disabled={model.isBusy}
            onClick={() => void actions.onConnectLocalWorld()}
            type="button"
            variant="outline"
          >
            <FolderOpen className="h-4 w-4" />
            Connect local world
          </Button>
          <Button
            className="h-9 justify-start gap-2"
            disabled={model.isBusy}
            onClick={actions.onToggleHostedConnection}
            type="button"
            variant="outline"
          >
            <Cloud className="h-4 w-4" />
            Connect hosted world
          </Button>
        </div>
        {model.isHostedConnectionOpen ? (
          <div className="grid gap-2 rounded-xl border border-border/70 bg-muted/20 p-3 sm:grid-cols-[1fr_auto] sm:items-end">
            <label
              className="grid gap-1 text-2xs font-medium text-muted-foreground"
              htmlFor={hostedCredentialInputId}
            >
              Hosted edit-share link
              <Input
                autoComplete="off"
                id={hostedCredentialInputId}
                onChange={(event) =>
                  actions.onHostedCredentialChange(event.target.value)
                }
                placeholder="https://manifest.shivai.space/world/edit/..."
                type="password"
                value={model.hostedCredential}
              />
            </label>
            <Button
              disabled={
                model.isHostedConnecting || !model.hostedCredential.trim()
              }
              onClick={() => void actions.onConnectHostedWorld()}
              size="sm"
              type="button"
            >
              {model.isHostedConnecting ? "Connecting..." : "Connect"}
            </Button>
            <p className="text-3xs leading-relaxed text-muted-foreground sm:col-span-2">
              The capability stays in this client. Buzz publishes only the
              stable world reference.
            </p>
          </div>
        ) : null}
      </section>

      <section className="grid gap-2 rounded-xl border border-border/70 bg-muted/15 p-3">
        <label
          className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground"
          htmlFor={publicReferenceInputId}
        >
          Or use a public read-only link
        </label>
        <div className="grid gap-2 sm:grid-cols-[1fr_auto]">
          <textarea
            className="min-h-9 resize-y rounded-md border border-input bg-background px-3 py-2 text-xs text-foreground shadow-sm outline-none placeholder:text-muted-foreground focus-visible:ring-1 focus-visible:ring-ring"
            id={publicReferenceInputId}
            onChange={(event) =>
              actions.onPublicReferenceChange(event.target.value)
            }
            placeholder="https://manifest.shivai.space/world/exports/..."
            rows={1}
            value={model.publicReference}
          />
          <Button
            disabled={!model.publicReference.trim() || model.isBusy}
            onClick={actions.onUsePublicReference}
            size="sm"
            type="button"
            variant="outline"
          >
            <Link2 className="h-3.5 w-3.5" />
            Use link
          </Button>
        </div>
        <p className="text-3xs leading-relaxed text-muted-foreground">
          You can also paste a copied Shivai view reference. Edit-share links
          and local paths are rejected.
        </p>
        {model.trustOrigin ? (
          <div
            aria-live="polite"
            className="grid gap-2 rounded-lg border border-amber-7 bg-amber-3 p-3"
          >
            <p className="text-xs font-medium text-amber-12">
              Trust this Shivai origin?
            </p>
            <p className="break-all text-3xs leading-relaxed text-amber-11">
              Buzz has not contacted {model.trustOrigin}. Trusting it allows
              this device to request the public view from that origin.
            </p>
            <div className="flex justify-end gap-2">
              <Button
                disabled={model.isTrustingOrigin}
                onClick={actions.onCancelOriginTrust}
                size="sm"
                type="button"
                variant="ghost"
              >
                Cancel
              </Button>
              <Button
                disabled={model.isTrustingOrigin}
                onClick={() => void actions.onTrustOrigin()}
                size="sm"
                type="button"
              >
                {model.isTrustingOrigin ? "Trusting..." : "Trust origin"}
              </Button>
            </div>
          </div>
        ) : null}
        {model.publicReferenceError ? (
          <p className="text-xs text-destructive">
            {model.publicReferenceError}
          </p>
        ) : null}
      </section>
    </>
  );
}

type WorldViewSettingsModel = {
  catalog: WorldViewCatalog | undefined;
  displayMode: WorldViewBinding["displayMode"];
  isCatalogFetching: boolean;
  label: string;
  referenceSelected: boolean;
  selectedView: WorldViewCatalogEntry | undefined;
  selectedViewQualifiedName: string;
};

type WorldViewSettingsActions = {
  onDisplayModeChange: (value: WorldViewBinding["displayMode"]) => void;
  onLabelChange: (value: string) => void;
  onViewChange: (value: string) => void;
};

type WorldViewSettingsProps = {
  actions: WorldViewSettingsActions;
  model: WorldViewSettingsModel;
};

function WorldViewSettings({ actions, model }: WorldViewSettingsProps) {
  const labelInputId = React.useId();
  const viewInputId = React.useId();

  return (
    <>
      <section className="grid gap-3 border-t border-border/60 pt-4 sm:grid-cols-2">
        <label
          className="grid gap-1 text-2xs font-medium text-muted-foreground"
          htmlFor={viewInputId}
        >
          View
          <select
            aria-label="View"
            className="h-9 rounded-md border border-input bg-background px-2 text-sm text-foreground shadow-sm disabled:cursor-not-allowed disabled:opacity-60"
            disabled={!model.referenceSelected || model.isCatalogFetching}
            id={viewInputId}
            onChange={(event) => actions.onViewChange(event.target.value)}
            required
            value={model.selectedViewQualifiedName}
          >
            <option value="">
              {model.isCatalogFetching
                ? "Loading views..."
                : model.referenceSelected
                  ? "Choose a view"
                  : "Choose a world first"}
            </option>
            {model.selectedViewQualifiedName &&
            model.catalog &&
            !model.selectedView ? (
              <option disabled value={model.selectedViewQualifiedName}>
                Previously selected view unavailable
              </option>
            ) : null}
            {model.catalog?.views.map((view) => (
              <option key={view.qualifiedName} value={view.qualifiedName}>
                {view.name} — {view.realm.name}
              </option>
            ))}
          </select>
          {model.catalog?.views.length === 0 ? (
            <span className="font-normal text-destructive">
              This world has no authored views.
            </span>
          ) : model.selectedView ? (
            <span className="truncate font-normal text-muted-foreground/80">
              {model.catalog?.worldQualifiedName} ·{" "}
              {model.selectedView.realm.name}
            </span>
          ) : null}
        </label>

        <label
          className="grid gap-1 text-2xs font-medium text-muted-foreground"
          htmlFor={labelInputId}
        >
          <span className="inline-flex items-baseline gap-1">
            Label
            <span className="font-normal opacity-70">Optional</span>
          </span>
          <Input
            id={labelInputId}
            onChange={(event) => actions.onLabelChange(event.target.value)}
            placeholder={model.selectedView?.name ?? "Launch board"}
            value={model.label}
          />
        </label>
      </section>

      <fieldset className="grid gap-1.5">
        <legend className="text-2xs font-medium text-muted-foreground">
          Initial display
        </legend>
        <div className="grid w-full grid-cols-2 rounded-lg bg-muted p-1 sm:w-64">
          {(["graph", "tasks"] as const).map((displayMode) => (
            <button
              aria-pressed={model.displayMode === displayMode}
              className={cn(
                "rounded-md px-3 py-1.5 text-xs font-medium capitalize text-muted-foreground transition-colors",
                model.displayMode === displayMode &&
                  "bg-background text-foreground shadow-sm",
              )}
              key={displayMode}
              onClick={() => actions.onDisplayModeChange(displayMode)}
              type="button"
            >
              {displayMode === "graph" ? "Graph" : "Tasks"}
            </button>
          ))}
        </div>
      </fieldset>
    </>
  );
}

type WorldAuthorityChoiceProps = {
  authority: WorldViewAuthority;
  isSelected: boolean;
  onSelect: () => void;
};

function WorldAuthorityChoice({
  authority,
  isSelected,
  onSelect,
}: WorldAuthorityChoiceProps) {
  const isLocal = authority.kind === "local-world-mirror-latest";
  const label = isLocal
    ? localWorldName(authority.sourceRoot)
    : "Hosted Shivai world";

  return (
    <button
      aria-pressed={isSelected}
      className={cn(
        "flex w-full items-center gap-3 px-3 py-2.5 text-left transition-colors hover:bg-muted/50",
        isSelected && "bg-primary/5",
      )}
      onClick={onSelect}
      title={isLocal ? authority.sourceRoot : authority.hostedWorldId}
      type="button"
    >
      <span
        className={cn(
          "grid h-8 w-8 shrink-0 place-items-center rounded-lg",
          isLocal ? "bg-amber-9/10 text-amber-11" : "bg-blue-9/10 text-blue-11",
        )}
      >
        {isLocal ? (
          <FolderOpen className="h-4 w-4" />
        ) : (
          <Cloud className="h-4 w-4" />
        )}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-foreground">
          {label}
        </span>
        <span className="block text-3xs text-muted-foreground">
          {isLocal ? "Published local world" : "Connected hosted world"}
        </span>
      </span>
      {isSelected ? <Check className="h-4 w-4 text-primary" /> : null}
    </button>
  );
}
