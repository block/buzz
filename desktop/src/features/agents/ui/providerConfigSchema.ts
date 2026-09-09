/** A provider-owned JSON-schema property supported by the generic renderer. */
export type ProviderConfigProperty = Record<string, unknown>;

/** A normalized provider configuration option. */
export type ProviderConfigOption = {
  label: string;
  value: string;
  [key: string]: unknown;
};

/** Return provider configuration properties in schema declaration order. */
export function providerConfigEntries(
  schema: Record<string, unknown>,
): [string, ProviderConfigProperty][] {
  const properties = schema.properties;
  if (
    !properties ||
    typeof properties !== "object" ||
    Array.isArray(properties)
  ) {
    return [];
  }
  return Object.entries(properties) as [string, ProviderConfigProperty][];
}

/** Return a schema property's scalar default as a form value. */
export function providerConfigDefault(
  property: ProviderConfigProperty,
): string {
  return typeof property.default === "string" ||
    typeof property.default === "number" ||
    typeof property.default === "boolean"
    ? String(property.default)
    : "";
}

/** Whether a property is visible for the current provider-owned configuration. */
export function providerConfigFieldVisible(
  property: ProviderConfigProperty,
  config: Record<string, string>,
): boolean {
  if (
    property["x-hide-when-no-options"] === true &&
    providerConfigOptions(property, config)?.length === 0
  ) {
    return false;
  }
  const condition = property["x-visible-when"];
  if (!condition || typeof condition !== "object" || Array.isArray(condition)) {
    return true;
  }
  const record = condition as Record<string, unknown>;
  if (typeof record.field !== "string") return true;
  const selected = config[record.field] ?? "";
  if (typeof record.equals === "string") return selected === record.equals;
  if (typeof record.not === "string") return selected !== record.not;
  return true;
}

/** Resolve a property's provider-owned bounded options for the current config. */
export function providerConfigOptions(
  property: ProviderConfigProperty,
  config: Record<string, string> = {},
): ProviderConfigOption[] | null {
  const multipleDependencies = property["x-options-by-fields"];
  if (
    multipleDependencies &&
    typeof multipleDependencies === "object" &&
    !Array.isArray(multipleDependencies)
  ) {
    const source = multipleDependencies as Record<string, unknown>;
    const fields = Array.isArray(source.fields)
      ? source.fields.filter(
          (field): field is string => typeof field === "string",
        )
      : [];
    const optionMap =
      source.options &&
      typeof source.options === "object" &&
      !Array.isArray(source.options)
        ? (source.options as Record<string, unknown>)
        : null;
    if (fields.length === 0 || fields.length > 4 || !optionMap) return [];
    const selected =
      optionMap[fields.map((field) => config[field] ?? "").join("|")];
    return Array.isArray(selected)
      ? selected.filter(isProviderConfigOption)
      : [];
  }

  const dependency = property["x-options-by-field"];
  if (
    dependency &&
    typeof dependency === "object" &&
    !Array.isArray(dependency)
  ) {
    const source = dependency as Record<string, unknown>;
    const field = typeof source.field === "string" ? source.field : null;
    const optionMap =
      source.options &&
      typeof source.options === "object" &&
      !Array.isArray(source.options)
        ? (source.options as Record<string, unknown>)
        : null;
    const selected = field ? optionMap?.[config[field] ?? ""] : null;
    let options = Array.isArray(selected)
      ? selected.filter(isProviderConfigOption)
      : [];
    const filter = property["x-option-filter"];
    if (filter && typeof filter === "object" && !Array.isArray(filter)) {
      const record = filter as Record<string, unknown>;
      if (
        typeof record.field === "string" &&
        typeof record.option_property === "string" &&
        config[record.field]
      ) {
        options = options.filter(
          (option) =>
            option[record.option_property as string] ===
            config[record.field as string],
        );
      }
    }
    return options;
  }

  if (Array.isArray(property.enum)) {
    const labels =
      property["x-enum-labels"] &&
      typeof property["x-enum-labels"] === "object" &&
      !Array.isArray(property["x-enum-labels"])
        ? (property["x-enum-labels"] as Record<string, unknown>)
        : {};
    return property.enum
      .filter((value): value is string | number =>
        ["string", "number"].includes(typeof value),
      )
      .map((value) => {
        const serialized = String(value);
        return {
          label:
            typeof labels[serialized] === "string"
              ? (labels[serialized] as string)
              : serialized,
          value: serialized,
        };
      });
  }

  if (Array.isArray(property.oneOf)) {
    const options = property.oneOf.flatMap((entry) => {
      if (!entry || typeof entry !== "object") return [];
      const option = entry as Record<string, unknown>;
      if (
        typeof option.const !== "string" &&
        typeof option.const !== "number"
      ) {
        return [];
      }
      return [
        {
          label:
            typeof option.title === "string"
              ? option.title
              : String(option.const),
          value: String(option.const),
        },
      ];
    });
    return options.length > 0 ? options : null;
  }

  if (property.type === "boolean") {
    return [
      { label: "Yes", value: "true" },
      { label: "No", value: "false" },
    ];
  }

  return null;
}

/**
 * Reconcile dependent schema values after one field changes.
 *
 * Hidden values return to their scalar default. Visible bounded values that no
 * longer exist return to a still-valid default or to an empty value.
 */
export function reconcileProviderConfig(
  entries: [string, ProviderConfigProperty][],
  config: Record<string, string>,
  key: string,
  value: string,
): Record<string, string> {
  const next = { ...config, [key]: value };
  for (const [dependentKey, dependentProperty] of entries) {
    if (!providerConfigFieldVisible(dependentProperty, next)) {
      next[dependentKey] = providerConfigDefault(dependentProperty);
      continue;
    }
    const options = providerConfigOptions(dependentProperty, next);
    if (
      options &&
      next[dependentKey] &&
      !options.some((option) => option.value === next[dependentKey])
    ) {
      const defaultValue = providerConfigDefault(dependentProperty);
      next[dependentKey] = options.some(
        (option) => option.value === defaultValue,
      )
        ? defaultValue
        : "";
    }
  }
  return next;
}

function isProviderConfigOption(value: unknown): value is ProviderConfigOption {
  return (
    Boolean(value) &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    typeof (value as ProviderConfigOption).value === "string" &&
    typeof (value as ProviderConfigOption).label === "string"
  );
}
