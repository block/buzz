import { createFileRoute } from "@tanstack/react-router";

import {
  type SettingsSection,
  isSettingsSection,
} from "@/features/settings/ui/SettingsPanels";

type SettingsRouteSearch = {
  section?: SettingsSection;
};

export function validateSettingsSearch(
  search: Record<string, unknown>,
): SettingsRouteSearch {
  if (search.section === "doctor") {
    return { section: "agents" };
  }

  if (search.section === "moderation") {
    return { section: "relay-admin" };
  }

  return {
    section: isSettingsSection(search.section) ? search.section : undefined,
  };
}

export const Route = createFileRoute("/settings")({
  validateSearch: validateSettingsSearch,
  component: SettingsRouteComponent,
});

// Settings renders at the AppShell level (it replaces the sidebar, top
// chrome, and router outlet wholesale), keyed off this route's presence —
// see AppShell. The outlet is unmounted while settings is open, so this
// component never actually renders.
function SettingsRouteComponent() {
  return null;
}
