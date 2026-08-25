import type {
  ChannelBinding,
  ChannelClassifyInput,
  ChannelFeaturePlugin,
} from "./types";

/**
 * Module-level plugin list, kept sorted by `priority` (ties preserve
 * registration order via `Array.prototype.sort`'s stability guarantee).
 *
 * Plugins are generic over their own binding type (`ChannelFeaturePlugin<T>`),
 * but the registry holds a heterogeneous mix, so entries are stored type-erased
 * as `ChannelFeaturePlugin<unknown>`. `registerChannelFeature` is the only
 * place that performs the erasure (via `unknown` double-cast, never `any`) —
 * every other module only ever sees the erased shape, which is exactly what
 * `classifyChannel`'s `ChannelBinding` result needs.
 */
let plugins: ChannelFeaturePlugin<unknown>[] = [];

/**
 * Register a channel-feature plugin. Duplicate `id`s are a no-op (warn +
 * ignore) rather than a throw, so a stray double-registration (e.g. a hot
 * reload or an accidental double barrel-import) can't crash app startup.
 */
export function registerChannelFeature<T>(
  plugin: ChannelFeaturePlugin<T>,
): void {
  if (plugins.some((existing) => existing.id === plugin.id)) {
    console.warn(
      `[channel-features] Duplicate channel feature id "${plugin.id}" — ignoring.`,
    );
    return;
  }
  plugins = [
    ...plugins,
    plugin as unknown as ChannelFeaturePlugin<unknown>,
  ].sort((a, b) => (a.priority ?? 0) - (b.priority ?? 0));
}

/** All registered channel-feature plugins, in classification order. */
export function getChannelPlugins(): ChannelFeaturePlugin<unknown>[] {
  return [...plugins];
}

/**
 * Classify `channel` against an explicit plugin list: the first plugin whose
 * `parseBinding` returns non-null wins (the list is assumed already in
 * priority order). `null` when none match. The primitive `classifyChannel`
 * builds on (passing the mutable registry); exposed for any caller that holds
 * a fixed plugin set of its own.
 */
export function classifyChannelWith(
  candidates: readonly ChannelFeaturePlugin<unknown>[],
  channel: ChannelClassifyInput,
): ChannelBinding | null {
  for (const plugin of candidates) {
    const value = plugin.parseBinding(channel);
    if (value !== null) {
      return { pluginId: plugin.id, value };
    }
  }
  return null;
}

/**
 * Classify `channel` against the registered plugins: the first plugin (in
 * priority order) whose `parseBinding` returns non-null wins. `null` when no
 * plugin matches — with the built-in `stream` catch-all plugin registered
 * (see `builtins.ts`), this only happens before that registration runs.
 */
export function classifyChannel(
  channel: ChannelClassifyInput,
): ChannelBinding | null {
  return classifyChannelWith(plugins, channel);
}

/** The matched plugin's glyph for `channel`, or `null` when nothing matched. */
export function channelGlyph(channel: ChannelClassifyInput) {
  const binding = classifyChannel(channel);
  if (!binding) return null;
  return (
    plugins.find((plugin) => plugin.id === binding.pluginId)?.glyph ?? null
  );
}

/** Test-only: reset the registry between test files/cases. */
export function __resetChannelFeatureRegistryForTests(): void {
  plugins = [];
}
