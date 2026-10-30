/**
 * Wire types for the restart-required config diff.
 *
 * These are camelCase / TS-idiomatic mirrors of the Rust `RestartDiffEntry`
 * serialized shape. The `field` is a dotted path derived from serde field
 * names (e.g. `"model"`, `"env.FOO"`); display labels are produced by
 * humanising the path — there is no per-field label map.
 */

/** A JSON-compatible value. Arrays are atomic leaves in the diff. */
export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };

/**
 * One change in a restart diff. The `kind` discriminant is a closed set of
 * five values; unknown `field` paths must render gracefully in the UI.
 */
export type RestartChange =
  | { kind: "value"; before: JsonValue; after: JsonValue }
  | { kind: "text"; before_chars: number | null; after_chars: number | null }
  | { kind: "masked"; before: string | null; after: string | null }
  | { kind: "added" }
  | { kind: "removed" };

export type RestartDiffEntry = {
  /** Dotted path derived from serde field names, e.g. "model", "env.FOO". */
  field: string;
  change: RestartChange;
};
