import { useEffect, useId, useRef, useState } from "react";
import { useRelaySelfQuery } from "@/features/moderation/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { Markdown } from "@/shared/ui/markdown";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_INTERACTION_CLOSE,
  KIND_INTERACTION_PROMPT,
  KIND_INTERACTION_RESPONSE,
  KIND_INTERACTION_STATE,
} from "@/shared/constants/kinds";
import {
  parseInteractionPrompt,
  parseInteractionState,
  type InteractionPrompt,
  type InteractionSummary,
} from "./protocol";

/** Opt-in card mounted on the relay's ordinary-message projection. */
export function InteractionCard({
  promptId,
  channelId,
  signer,
  currentPubkey,
  fallback,
}: {
  promptId: string;
  channelId: string;
  signer?: string;
  currentPubkey?: string;
  fallback: string;
}) {
  const relayQuery = useRelaySelfQuery();
  const relay = relayQuery.data;
  const [prompt, setPrompt] = useState<InteractionPrompt | null>(null);
  const [state, setState] = useState<InteractionSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [pending, setPending] = useState(false);
  const [choices, setChoices] = useState<string[]>([]);
  const [values, setValues] = useState<Record<string, string>>({});
  const [dirty, setDirty] = useState(false);
  const generation = useRef(0);
  const inputId = useId();
  const myAnswer = state?.responders.find((r) => r.pubkey === currentPubkey);
  const expired = prompt ? Date.now() / 1000 >= prompt.deadline : false;
  const closed = state?.status === "closed" || expired;
  const disabled = pending || closed || !state || !currentPubkey;

  useEffect(() => {
    if (!dirty) setChoices(myAnswer?.choices ?? []);
  }, [myAnswer, dirty]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: retry explicitly restarts a failed subscription.
  useEffect(() => {
    const version = ++generation.current;
    let unsubscribe: (() => void) | undefined;
    let cancelled = false;
    setPrompt(null);
    setState(null);
    setError(null);
    setPending(false);
    setDirty(false);
    setValues({});
    if (!relay) return;
    if (signer !== relay || !/^[a-f0-9]{64}$/.test(promptId)) {
      setError("This message is not a verified relay interaction.");
      return;
    }
    const receive = (event: RelayEvent) => {
      if (cancelled || generation.current !== version) return;
      const next = parseInteractionState(event, promptId, channelId, relay);
      if (next)
        setState((previous) =>
          !previous || next.revision > previous.revision ? next : previous,
        );
    };
    const load = async () => {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_INTERACTION_PROMPT],
        ids: [promptId],
        "#h": [channelId],
        limit: 1,
      });
      if (cancelled || generation.current !== version) return;
      if (!events[0]) throw new Error("This prompt is unavailable.");
      setPrompt(parseInteractionPrompt(events[0], promptId, channelId));
      // One history + live subscription; no gap between a backfill and watching.
      const stop = await relayClient.subscribeLive(
        {
          kinds: [KIND_INTERACTION_STATE],
          authors: [relay],
          "#d": [promptId],
          "#h": [channelId],
          limit: 1,
        },
        receive,
      );
      if (cancelled || generation.current !== version) stop();
      else unsubscribe = stop;
    };
    void load().catch((reason: unknown) => {
      if (!cancelled && generation.current === version)
        setError(String(reason));
    });
    return () => {
      cancelled = true;
      generation.current++;
      unsubscribe?.();
    };
  }, [promptId, channelId, signer, relay, retry]);

  async function submit(selected: string[], close = false) {
    if (!prompt || !relay || disabled) return;
    const version = generation.current;
    const tags = [
      ["h", channelId],
      ["e", promptId, "", "prompt"],
    ];
    if (!close) {
      for (const choice of selected) tags.push(["choice", choice]);
      for (const field of prompt.fields) {
        const value = values[field.id];
        if (value !== undefined && value !== "")
          tags.push(["value", field.id, value]);
      }
    }
    setPending(true);
    setError(null);
    try {
      const response = await signRelayEvent({
        kind: close ? KIND_INTERACTION_CLOSE : KIND_INTERACTION_RESPONSE,
        content: "",
        tags,
        createdAt: Math.max(
          Math.floor(Date.now() / 1000),
          (myAnswer?.created_at ?? 0) + 1,
        ),
      });
      if (generation.current !== version) return;
      await relayClient.publishEvent(
        response,
        "Timed out sending your answer.",
        "Your answer was not accepted.",
      );
      const events = await relayClient.fetchEvents({
        kinds: [KIND_INTERACTION_STATE],
        authors: [relay],
        "#d": [promptId],
        "#h": [channelId],
        limit: 1,
      });
      if (generation.current !== version) return;
      const next =
        events[0] &&
        parseInteractionState(events[0], promptId, channelId, relay);
      if (next)
        setState((previous) =>
          !previous || next.revision > previous.revision ? next : previous,
        );
      setDirty(false);
    } catch (reason) {
      if (generation.current === version) setError(String(reason));
    } finally {
      if (generation.current === version) setPending(false);
    }
  }

  if (!prompt)
    return (
      <div className="space-y-2">
        <p className="whitespace-pre-wrap text-message">{fallback}</p>
        {(error || relayQuery.isError) && (
          <p role="alert" className="text-sm text-red-500">
            {error ?? "Unable to verify the relay identity."}{" "}
            <button
              type="button"
              className="underline"
              onClick={() => {
                if (relayQuery.isError) void relayQuery.refetch();
                setRetry((r) => r + 1);
              }}
            >
              Retry
            </button>
          </p>
        )}
      </div>
    );
  const listed =
    prompt.event.tags.find((t) => t[0] === "responders")?.[1] === "listed";
  const eligible =
    !listed ||
    prompt.event.tags.some((t) => t[0] === "p" && t[1] === currentPubkey);
  const buttonClass =
    "rounded-md border px-3 py-2 text-sm focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 disabled:opacity-50";
  return (
    <section
      className="space-y-3 rounded-lg border border-primary/30 bg-primary/5 p-3"
      aria-label="Interaction"
      data-testid="interaction-card"
    >
      <Markdown content={prompt.event.content} />
      <p className="text-xs text-muted-foreground">
        {prompt.type === "poll" ? "Poll" : "Request for input"} · Answers are
        visible in this channel
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const button = (e.nativeEvent as SubmitEvent).submitter;
          void submit(
            prompt.type === "buttons" && button instanceof HTMLButtonElement
              ? [button.value]
              : choices,
          );
        }}
      >
        <fieldset disabled={disabled || !eligible} className="space-y-3">
          <legend className="sr-only">Your answer</legend>
          {prompt.type === "poll" && (
            <div className="space-y-2">
              {prompt.options.map((option) => (
                <label
                  key={option.id}
                  className="flex items-center gap-2 text-sm"
                >
                  <input
                    type={prompt.max === 1 ? "radio" : "checkbox"}
                    name={`${inputId}-choice`}
                    checked={choices.includes(option.id)}
                    onChange={(e) => {
                      setDirty(true);
                      setChoices(
                        prompt.max === 1
                          ? [option.id]
                          : e.target.checked
                            ? [...choices, option.id]
                            : choices.filter((c) => c !== option.id),
                      );
                    }}
                  />
                  {option.label}
                  <span className="ml-auto tabular-nums">
                    {state?.tally[option.id] ?? 0}
                  </span>
                </label>
              ))}
            </div>
          )}
          {prompt.fields.map((field) => (
            <div key={field.id} className="space-y-1">
              <label
                className="block text-sm"
                htmlFor={`${inputId}-${field.id}`}
              >
                {field.label}
                {field.required ? " (required)" : " (optional)"}
              </label>
              {field.type === "select" || field.type === "boolean" ? (
                <select
                  id={`${inputId}-${field.id}`}
                  className="w-full rounded border bg-background p-2 text-sm"
                  required={field.required}
                  value={values[field.id] ?? ""}
                  onChange={(e) =>
                    setValues({ ...values, [field.id]: e.target.value })
                  }
                >
                  <option value="">Choose…</option>
                  {(field.type === "boolean"
                    ? [
                        { id: "true", label: "Yes" },
                        { id: "false", label: "No" },
                      ]
                    : field.options
                  ).map((option) => (
                    <option key={option.id} value={option.id}>
                      {option.label}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  id={`${inputId}-${field.id}`}
                  className="w-full rounded border bg-background p-2 text-sm"
                  type={field.type}
                  step={field.type === "number" ? "any" : undefined}
                  maxLength={4096}
                  required={field.required}
                  value={values[field.id] ?? ""}
                  onChange={(e) =>
                    setValues({ ...values, [field.id]: e.target.value })
                  }
                />
              )}
            </div>
          ))}
          {prompt.type === "buttons" ? (
            <div className="flex flex-wrap gap-2">
              {prompt.options.map((option) => (
                <button
                  key={option.id}
                  type="submit"
                  value={option.id}
                  className={`${buttonClass} ${option.style === "danger" ? "border-red-500 text-red-500" : option.style === "primary" ? "bg-primary text-primary-foreground" : "bg-background"}`}
                >
                  {option.label}
                  {myAnswer?.choices.includes(option.id) ? " ✓" : ""}
                </button>
              ))}
            </div>
          ) : (
            <button
              type="submit"
              className={buttonClass}
              disabled={
                choices.length < prompt.min || choices.length > prompt.max
              }
            >
              {myAnswer ? "Update answer" : "Submit answer"}
            </button>
          )}
        </fieldset>
      </form>
      <p
        className="text-xs text-muted-foreground"
        role="status"
        aria-live="polite"
      >
        {pending
          ? "Sending answer…"
          : closed
            ? `Closed${state?.winner ? ` · ${prompt.options.find((o) => o.id === state.winner)?.label ?? state.winner}` : ""}${state?.close_reason ? ` (${state.close_reason})` : ""}`
            : !state
              ? "Loading decision state…"
              : `${state.responders.length} answered${myAnswer ? " · Your answer is recorded" : ""}`}
      </p>
      {!eligible && (
        <p className="text-xs text-muted-foreground">
          This request is addressed to other members.
        </p>
      )}
      {prompt.event.pubkey === currentPubkey && !closed && (
        <button
          type="button"
          disabled={disabled}
          className="text-xs underline"
          onClick={() => void submit([], true)}
        >
          Close request
        </button>
      )}
      {(error || !state) && (
        <p role="alert" className="text-sm text-red-500">
          {error}{" "}
          <button
            type="button"
            className="underline"
            onClick={() => setRetry((r) => r + 1)}
          >
            Reload request
          </button>
        </p>
      )}
    </section>
  );
}
