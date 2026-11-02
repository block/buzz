import * as React from "react";
import { useNavigate } from "@tanstack/react-router";

import { useCustomerRecords } from "@/features/cofounder/customerHooks";
import { useCustomerRecordCapabilities } from "@/features/cofounder/customerRecordAccess";
import { formatZar } from "./BusinessCommon";
import { BusinessLogoMark } from "./BusinessLogoMark";
import { useDiscoveryCampaign } from "./discoveryProvider";
import { AddLeadDialog } from "./LeadDialogs";
import { LeadPill, PlusGlyph, RunnerMark, SearchGlyph } from "./LeadParts";
import {
  canEdit,
  labelForStage,
  leadInitial,
  STAGES,
  toneForStage,
  type LeadContext,
} from "./leadShared";
import {
  findWhatsAppContactForLead,
  getWhatsAppLeadStatus,
  useWhatsAppLeadData,
} from "@/features/whatsapp-outreach/whatsappLeadStatus";

import "./colony3-leads.css";

export function LeadsScreen(context: LeadContext) {
  const navigate = useNavigate();
  const { campaign } = useDiscoveryCampaign();
  const customerAccess = useCustomerRecordCapabilities();
  const customerRecordsQuery = useCustomerRecords(customerAccess.enabled);
  const { contactsQuery, inBatchLeadIds } = useWhatsAppLeadData();
  const [query, setQuery] = React.useState("");
  const [stage, setStage] = React.useState("all");
  const [whatsappFilter, setWhatsappFilter] = React.useState("all");
  const [dialogOpen, setDialogOpen] = React.useState(false);
  const canWrite = canEdit(context.role);
  const customerRecordByPartyId = React.useMemo(
    () =>
      new Map(
        (customerRecordsQuery.data ?? []).map((record) => [
          record.partyId,
          record,
        ]),
      ),
    [customerRecordsQuery.data],
  );
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const activeProspects = context.records.prospects.filter(
    ({ record }) => record.status === "active",
  );
  const prospects = activeProspects.filter(({ record }) => {
    const prospect = record.prospect;
    const matchesQuery =
      !normalizedQuery ||
      [
        prospect.party.displayName,
        prospect.contactName ?? "",
        prospect.industry,
        prospect.vertical,
        prospect.location ?? "",
      ]
        .join(" ")
        .toLocaleLowerCase()
        .includes(normalizedQuery);
    const customerRecord = customerRecordByPartyId.get(prospect.party.partyId);
    const matchesStage =
      stage === "all" ||
      (customerRecord?.stage === "client"
        ? stage === "client"
        : stage === "new"
          ? prospect.qualification === "unreviewed"
          : prospect.stage === stage);
    const contact = findWhatsAppContactForLead(
      record.prospectId,
      contactsQuery.data ?? [],
    );
    const matchesWhatsApp =
      whatsappFilter === "all" ||
      getWhatsAppLeadStatus(contact, inBatchLeadIds.has(record.prospectId)) ===
        whatsappFilter;
    return matchesQuery && matchesStage && matchesWhatsApp;
  });
  const runner = campaign?.runBy ?? null;

  return (
    <main className="c3l-page" data-testid="w10-leads-page">
      <div className="c3l-wrap c3l-wrap-leads">
        <div className="c3l-head">
          <div className="c3l-head-text">
            <h1 className="c3l-title">Leads</h1>
            <span className="c3l-subtitle c3l-subtitle-inline">
              {activeProspects.length}{" "}
              {activeProspects.length === 1 ? "lead" : "leads"}
              {runner ? (
                <>
                  {" · sourced by "}
                  <RunnerMark runner={runner} size={22} />
                  {runner.name}’s Discovery campaign
                </>
              ) : null}
            </span>
          </div>
          <div className="c3l-head-actions">
            {canWrite ? (
              <button
                className="c3l-btn c3l-btn-outline"
                onClick={() => setDialogOpen(true)}
                type="button"
              >
                <PlusGlyph />
                Add lead
              </button>
            ) : null}
            <button
              className="c3l-btn c3l-btn-primary"
              onClick={() => void navigate({ to: "/discovery" })}
              type="button"
            >
              Find prospects <span aria-hidden="true">→</span>
            </button>
          </div>
        </div>

        <div className="c3l-filterbar">
          <label className="c3l-search">
            <SearchGlyph />
            <input
              aria-label="Search leads"
              onChange={(event) => setQuery(event.currentTarget.value)}
              placeholder="Search leads…"
              type="search"
              value={query}
            />
          </label>
          <select
            aria-label="Stage"
            className="c3l-select c3l-select-filter"
            id="w10-lead-stage-filter"
            onChange={(event) => setStage(event.currentTarget.value)}
            value={stage}
          >
            <option value="all">All stages</option>
            <option value="new">New</option>
            <option value="client">Client</option>
            {STAGES.map((item) => (
              <option key={item.value} value={item.value}>
                {item.label}
              </option>
            ))}
          </select>
          <select
            aria-label="WhatsApp status"
            className="c3l-select c3l-select-filter"
            onChange={(event) => setWhatsappFilter(event.currentTarget.value)}
            value={whatsappFilter}
          >
            <option value="all">Any WhatsApp status</option>
            <option value="not-contacted">Not contacted</option>
            <option value="in-batch">In batch</option>
            <option value="sent">Sent</option>
            <option value="replied">Replied</option>
            <option value="opted-out">Opted out</option>
          </select>
        </div>

        {prospects.length ? (
          <div className="c3l-card c3l-table-wrap">
            <table aria-label="Leads" className="c3l-table">
              <thead>
                <tr className="c3l-row c3l-row-head">
                  <th>Business</th>
                  <th>Contact</th>
                  <th>Stage</th>
                  <th>Service / month</th>
                  <th>Next step</th>
                </tr>
              </thead>
              <tbody>
                {prospects.map(({ record, eventId }) => {
                  const prospect = record.prospect;
                  const customerRecord = customerRecordByPartyId.get(
                    prospect.party.partyId,
                  );
                  return (
                    <tr className="c3l-row" key={eventId}>
                      <td className="c3l-cell-business">
                        <div className="c3l-business-identity">
                          <BusinessLogoMark
                            className="c3l-mark"
                            name={prospect.party.displayName}
                            website={prospect.website}
                          />
                          <span className="c3l-business-text">
                            <strong>{prospect.party.displayName}</strong>
                            <small>
                              {prospect.vertical} ·{" "}
                              {prospect.location ?? "Location not set"}
                            </small>
                          </span>
                        </div>
                      </td>
                      <td className="c3l-cell-contact">
                        {prospect.contactName ? (
                          <>
                            <span aria-hidden="true" className="c3l-dot-mark">
                              {leadInitial(prospect.contactName)}
                            </span>
                            {prospect.contactName}
                          </>
                        ) : (
                          <span className="c3l-muted c3l-empty-value">
                            Not set
                          </span>
                        )}
                      </td>
                      <td>
                        {customerRecord?.stage === "client" ? (
                          <LeadPill tone="green">Client</LeadPill>
                        ) : (
                          <LeadPill
                            tone={toneForStage(
                              prospect.stage,
                              prospect.qualification,
                            )}
                          >
                            {labelForStage(
                              prospect.stage,
                              prospect.qualification,
                              "New",
                            )}
                          </LeadPill>
                        )}
                      </td>
                      <td className="c3l-cell-amount">
                        {prospect.potentialMonthlyValueMinor === null ||
                        prospect.potentialMonthlyValueMinor === undefined ? (
                          <span className="c3l-muted c3l-empty-value">
                            Not set
                          </span>
                        ) : (
                          formatZar(prospect.potentialMonthlyValueMinor)
                        )}
                      </td>
                      <td className="c3l-cell-next-step">
                        <a
                          className="c3l-next-step"
                          href={`#/sales/lead/${record.prospectId}`}
                        >
                          Review lead
                        </a>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        ) : (
          <div className="c3l-card c3l-empty">
            <h2>
              {activeProspects.length
                ? "No leads match these filters."
                : "No leads yet"}
            </h2>
            <p>
              {activeProspects.length
                ? "Try a different name, location or stage."
                : "Find a business that could benefit from your service, or add a lead yourself."}
            </p>
            {activeProspects.length ? (
              <button
                className="c3l-btn c3l-btn-outline"
                onClick={() => {
                  setQuery("");
                  setStage("all");
                }}
                type="button"
              >
                Clear filters
              </button>
            ) : (
              <button
                className="c3l-btn c3l-btn-primary"
                onClick={() => void navigate({ to: "/discovery" })}
                type="button"
              >
                Explore Discovery
              </button>
            )}
          </div>
        )}
      </div>

      <AddLeadDialog
        canWrite={canWrite}
        context={context}
        onOpenChange={setDialogOpen}
        open={dialogOpen}
      />
    </main>
  );
}
