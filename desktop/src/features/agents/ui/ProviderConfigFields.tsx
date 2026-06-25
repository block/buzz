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
      {entries.map(([key, prop]) => (
        <div key={key} className="space-y-1.5">
          <label
            className="text-sm font-medium"
            htmlFor={`provider-cfg-${key}`}
          >
            {typeof prop.title === "string" ? prop.title : key}
            {required.has(key) ? (
              <span className="ml-1 text-destructive">*</span>
            ) : null}
          </label>
          <Input
            id={`provider-cfg-${key}`}
            onChange={(e) => onChange({ ...config, [key]: e.target.value })}
            placeholder={
              typeof prop.description === "string" ? prop.description : ""
            }
            value={
              config[key] ??
              (typeof prop.default === "string" ? prop.default : "")
            }
          />
          {typeof prop.description === "string" ? (
            <p className="text-xs text-muted-foreground">{prop.description}</p>
          ) : null}
        </div>
      ))}
    </div>
  );
}
