import { Input } from "@/shared/ui/input";
import { PersonaDropdownField } from "./PersonaDropdownField";
import {
  providerConfigDefault,
  providerConfigEntries,
  providerConfigFieldVisible,
  providerConfigOptions,
  reconcileProviderConfig,
} from "./providerConfigSchema";

/** Whether a schema needs the provider-owned extended field presentation. */
export function providerSchemaUsesExtendedPresentation(
  schema: Record<string, unknown>,
): boolean {
  return providerConfigEntries(schema).some(([, property]) => {
    const type = property.type;
    return (
      type === "integer" ||
      type === "number" ||
      type === "boolean" ||
      property.readOnly === true ||
      Array.isArray(property.enum) ||
      Array.isArray(property.oneOf) ||
      Object.keys(property).some((key) => key.startsWith("x-"))
    );
  });
}

/** Render provider-owned JSON-schema extensions without provider-specific IDs. */
export function ProviderConfigSchemaFields({
  schema,
  config,
  onChange,
}: {
  schema: Record<string, unknown>;
  config: Record<string, string>;
  onChange: (config: Record<string, string>) => void;
}) {
  const entries = providerConfigEntries(schema);
  const required = new Set<string>(
    Array.isArray(schema.required)
      ? schema.required.filter(
          (value): value is string => typeof value === "string",
        )
      : [],
  );

  if (entries.length === 0) return null;

  const updateConfig = (key: string, value: string) => {
    onChange(reconcileProviderConfig(entries, config, key, value));
  };

  return (
    <div className="space-y-3">
      {entries.map(([key, property]) => {
        if (!providerConfigFieldVisible(property, config)) return null;
        const options = providerConfigOptions(property, config);
        const value = config[key] ?? providerConfigDefault(property);

        return (
          <div key={key} className="space-y-1.5">
            <label
              className="text-sm font-medium"
              htmlFor={`provider-cfg-${key}`}
            >
              {typeof property.title === "string" ? property.title : key}
              {required.has(key) ? (
                <span className="ml-1 text-destructive">*</span>
              ) : null}
            </label>
            {property.readOnly === true ? (
              <p
                className="rounded-xl border bg-muted/40 px-3 py-2 text-sm text-muted-foreground"
                id={`provider-cfg-${key}`}
              >
                {value}
              </p>
            ) : options ? (
              <PersonaDropdownField
                id={`provider-cfg-${key}`}
                onValueChange={(nextValue) => updateConfig(key, nextValue)}
                options={options}
                placeholder={`Choose ${
                  typeof property.title === "string"
                    ? property.title.toLowerCase()
                    : key
                }`}
                value={value}
              />
            ) : (
              <Input
                id={`provider-cfg-${key}`}
                max={
                  typeof property.maximum === "number"
                    ? property.maximum
                    : undefined
                }
                min={
                  typeof property.minimum === "number"
                    ? property.minimum
                    : undefined
                }
                onChange={(event) => updateConfig(key, event.target.value)}
                placeholder={
                  typeof property.description === "string"
                    ? property.description
                    : ""
                }
                step={property.type === "integer" ? 1 : undefined}
                type={
                  property.type === "integer" || property.type === "number"
                    ? "number"
                    : "text"
                }
                value={value}
              />
            )}
            {typeof property.description === "string" ? (
              <p className="text-xs text-muted-foreground">
                {property.description}
              </p>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
