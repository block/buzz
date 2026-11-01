// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
function commandLooksLikePath(command: string) {
  const trimmed = command.trim();
  return (
    trimmed.startsWith(".") ||
    trimmed.startsWith("~") ||
    trimmed.includes("/") ||
    trimmed.includes("\\")
  );
}

export function describeResolvedCommand(command: string, resolvedPath: string) {
  const normalized = resolvedPath.replace(/\\/g, "/");

  if (normalized.includes("/target/release/")) {
    return uiT("workspace release build");
  }
  if (normalized.includes("/target/debug/")) {
    return uiT("workspace debug build");
  }

  if (commandLooksLikePath(command)) {
    return uiT("custom command");
  }

  return uiT("installed on PATH");
}

export function describeLogFile(path: string) {
  const normalized = path.replace(/\\/g, "/");
  const basename = normalized.split("/").pop() ?? path;

  if (!basename.endsWith(".log")) {
    return uiT("local harness log");
  }

  const stem = basename.slice(0, -4);
  if (stem.length <= 18) {
    return basename;
  }

  return `${stem.slice(0, 8)}…${stem.slice(-6)}.log`;
}
