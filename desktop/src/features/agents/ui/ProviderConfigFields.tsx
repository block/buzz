import { Input } from "@/shared/ui/input";

/// Coerce string config values to their schema-declared types (number, boolean).
/// Providers receive JSON — sending "3" instead of 3 for an integer field breaks
/// typed config parsing on the provider side.
export function coerceConfigValues(
  config: Record<string, string>,
  schema: Record<string, unknown> | undefined,
): Record<string, unknown> {
  if (!schema) return { ...config };
  const properties = ((schema as Record<string, unknown>)?.properties ??
    {}) as Record<string, Record<string, unknown>>;
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(config)) {
    const prop = properties[key] as Record<string, unknown> | undefined;
    const schemaType = prop?.type;
    if ((schemaType === "integer" || schemaType === "number") && value !== "") {
      const num = Number(value);
      result[key] = Number.isNaN(num) ? value : num;
    } else if (schemaType === "boolean") {
      result[key] = value === "true";
    } else {
      result[key] = value;
    }
  }
  return result;
}

function enumOptions(prop: Record<string, unknown>): {
  value: string;
  label: string;
}[] {
  const raw = prop.enum;
  if (!Array.isArray(raw) || raw.length === 0) return [];
  const labels =
    (prop.enumLabels as Record<string, string> | undefined) ??
    (prop["x-enumLabels"] as Record<string, string> | undefined) ??
    {};
  return raw.map((entry) => {
    const value = entry == null ? "" : String(entry);
    const label = labels[value] ?? (value === "" ? "Default" : value);
    return { value, label };
  });
}

export function ProviderConfigFields({
  schema,
  config,
  onChange,
}: {
  schema: Record<string, unknown>;
  config: Record<string, string>;
  onChange: (config: Record<string, string>) => void;
}) {
  const properties = (schema as Record<string, unknown>)?.properties ?? {};
  const required = new Set<string>(
    ((schema as Record<string, unknown>)?.required as string[]) ?? [],
  );

  const entries = Object.entries(properties) as [
    string,
    Record<string, unknown>,
  ][];

  if (entries.length === 0) {
    return null;
  }

  return (
    <div className="space-y-3">
      {entries.map(([key, prop]) => {
        const options = enumOptions(prop);
        const defaultValue =
          prop.default == null
            ? ""
            : typeof prop.default === "boolean"
              ? prop.default
                ? "true"
                : "false"
              : String(prop.default);
        const value = config[key] ?? defaultValue;
        const fieldId = `provider-cfg-${key}`;
        const title = typeof prop.title === "string" ? prop.title : key;
        const description =
          typeof prop.description === "string" ? prop.description : null;

        return (
          <div key={key} className="space-y-1.5">
            <label className="text-sm font-medium" htmlFor={fieldId}>
              {title}
              {required.has(key) ? (
                <span className="ml-1 text-destructive">*</span>
              ) : null}
            </label>
            {options.length > 0 ? (
              <select
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
                id={fieldId}
                onChange={(e) => onChange({ ...config, [key]: e.target.value })}
                value={value}
              >
                {options.map((option) => (
                  <option key={option.value || "__empty"} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            ) : prop.type === "boolean" ? (
              <select
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
                id={fieldId}
                onChange={(e) => onChange({ ...config, [key]: e.target.value })}
                value={value === "true" ? "true" : "false"}
              >
                <option value="false">No</option>
                <option value="true">Yes</option>
              </select>
            ) : (
              <Input
                id={fieldId}
                onChange={(e) => onChange({ ...config, [key]: e.target.value })}
                placeholder={description ?? ""}
                value={value}
              />
            )}
            {description ? (
              <p className="text-xs text-muted-foreground">{description}</p>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
