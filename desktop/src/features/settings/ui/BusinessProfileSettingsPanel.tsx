import * as React from "react";
import { toast } from "sonner";

import { WorkspaceMark } from "@/features/communities/ui/WorkspaceMark";
import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import { CommunityIconSettingsCard } from "@/features/communities/ui/CommunityIconSettingsCard";
import { useCommunities } from "@/features/communities/useCommunities";
import { writeTextToClipboard } from "@/shared/lib/clipboard";
import { CopyIcon } from "./settingsIcons";

/** "ws://lerato.colony.example:443/" reads "lerato.colony.example". */
function communityAddress(relayUrl: string | undefined): string {
  if (!relayUrl) return "";
  try {
    return new URL(relayUrl).host;
  } catch {
    return relayUrl;
  }
}

/**
 * Business > Workspace: the business icon, its name and its address. Owners
 * and administrators can rename it; everyone else reads it.
 */
export function BusinessProfileSettingsPanel() {
  const { activeCommunity, updateCommunity } = useCommunities();
  const membershipQuery = useMyRelayMembershipLookupQuery();
  const role = membershipQuery.data?.membership?.role;
  const canEdit =
    membershipQuery.data?.membershipRequired === false ||
    role === "owner" ||
    role === "admin";
  const [name, setName] = React.useState(activeCommunity?.name ?? "");
  const [editingIcon, setEditingIcon] = React.useState(false);
  const [saved, setSaved] = React.useState(false);
  const address = communityAddress(activeCommunity?.relayUrl);
  React.useEffect(() => {
    setName(activeCommunity?.name ?? "");
    setSaved(false);
  }, [activeCommunity?.name]);

  if (!activeCommunity) {
    return (
      <section
        className="cs-card cs-card-body"
        data-testid="settings-business-profile"
      >
        <p className="cs-section-note">No active workspace is connected.</p>
      </section>
    );
  }
  const community = activeCommunity;
  const trimmed = name.trim();

  function save(event: React.FormEvent) {
    event.preventDefault();
    if (!canEdit || !trimmed || trimmed === community.name) return;
    const result = updateCommunity(community.id, { name: trimmed });
    if (result.kind === "not-found" || result.kind === "duplicate-relay") {
      toast.error("Couldn’t rename the workspace.");
      return;
    }
    setSaved(true);
  }

  async function copyAddress() {
    try {
      await writeTextToClipboard(address);
      toast.success("Community address copied");
    } catch {
      toast.error("Couldn’t copy the community address.");
    }
  }

  return (
    <section
      aria-labelledby="business-workspace-title"
      className="cs-card"
      data-testid="settings-business-profile"
    >
      <form className="cs-card-body cs-card-body-tight" onSubmit={save}>
        <h2 className="cs-card-title" id="business-workspace-title">
          Workspace
        </h2>
        <div className="cs-photo-row" data-gap="small">
          <WorkspaceMark
            className="cs-workspace-mark"
            communityId={community.id}
            dataTestId="business-profile-workspace-mark"
            dot={<i />}
            name={community.name}
            relayUrl={community.relayUrl}
          />
          <div className="cs-photo-copy" data-gap="small">
            <button
              aria-expanded={editingIcon}
              className="cs-button"
              data-testid="business-change-icon"
              disabled={!canEdit}
              onClick={() => setEditingIcon((open) => !open)}
              type="button"
            >
              Change icon
            </button>
            <span className="cs-help">
              Shown in the sidebar and the community switcher.
            </span>
          </div>
        </div>
        {editingIcon && canEdit ? <CommunityIconSettingsCard compact /> : null}
        <div className="cs-form-grid">
          <div className="cs-field">
            <label className="cs-label" htmlFor="business-name">
              Workspace name
            </label>
            <input
              className="cs-input"
              data-testid="business-name"
              id="business-name"
              onChange={(event) => {
                setName(event.target.value);
                setSaved(false);
              }}
              readOnly={!canEdit}
              value={name}
            />
          </div>
          <div className="cs-field">
            <label className="cs-label" htmlFor="business-address">
              Community address
            </label>
            <span className="cs-address-row">
              <input
                className="cs-input"
                data-testid="business-address"
                id="business-address"
                readOnly
                value={address}
              />
              <button
                aria-label="Copy community address"
                className="cs-icon-button cs-icon-button-large"
                data-testid="business-copy-address"
                onClick={() => void copyAddress()}
                type="button"
              >
                <CopyIcon size={17} />
              </button>
            </span>
          </div>
        </div>
        {canEdit ? (
          <div className="cs-save-row">
            <button
              className="cs-primary"
              data-testid="business-save"
              disabled={!trimmed}
              type="submit"
            >
              Save
            </button>
            {saved ? (
              <span className="cs-help" role="status">
                Saved.
              </span>
            ) : null}
          </div>
        ) : null}
      </form>
    </section>
  );
}
