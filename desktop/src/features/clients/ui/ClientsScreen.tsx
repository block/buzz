import * as React from "react";
import { Link, useNavigate } from "@tanstack/react-router";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useCustomerRecords } from "@/features/cofounder/customerHooks";
import { useCofounderMode } from "@/features/cofounder/customerRecordAccess";

import type { WorkItemHead } from "@/features/clients/lib/businessRecords";
import { usePeople } from "@/features/clients/lib/usePeople";
import { useAllWorkItemHeadsQuery } from "@/features/clients/useBusinessRecords";
import {
  BriefcaseIcon,
  ChevronRightIcon,
  clientInitial,
  ClientStatusPill,
  PersonAvatar,
  WorkStatusPill,
} from "@/features/clients/ui/BusinessBits";
import {
  ClientWorkspace,
  RecordMessage,
} from "@/features/clients/ui/ClientWorkspace";

/** The work item a client row shows: the one that most needs a person. */
const WORK_PRIORITY = ["review", "blocked", "active", "paused", "complete"];

type ClientDirectoryRow = {
  event: { id: string };
  value: {
    partyId: string;
    clientId: string | null;
    displayName: string;
    status: string;
  };
};

export function sharedWorkForClient(
  work: readonly { value: WorkItemHead }[],
  clientId: string,
): WorkItemHead | null {
  const own = work
    .map(({ value }) => value)
    .filter((value) => value.clientId === clientId);
  const rank = (status: string) => {
    const index = WORK_PRIORITY.indexOf(status);
    return index === -1 ? WORK_PRIORITY.length : index;
  };
  return [...own].sort((a, b) => rank(a.status) - rank(b.status))[0] ?? null;
}

export function ClientsScreen() {
  const workQuery = useAllWorkItemHeadsQuery();
  const { goClient } = useAppNavigation();
  const navigate = useNavigate();
  const cofounderMode = useCofounderMode();
  const cofounderOn = cofounderMode.enabled;
  const clientsQuery = workQuery.directoryQuery;
  const customerRecordsQuery = useCustomerRecords(cofounderOn);
  const records = React.useMemo<ClientDirectoryRow[]>(() => {
    const byPartyId = new Map<string, ClientDirectoryRow>();
    for (const { event, value } of clientsQuery.data ?? []) {
      byPartyId.set(value.partyId, {
        event,
        value: {
          partyId: value.partyId,
          clientId: value.clientId,
          displayName: value.displayName,
          status: value.status,
        },
      });
    }
    if (cofounderOn) {
      for (const record of customerRecordsQuery.data ?? []) {
        if (record.stage !== "client") continue;
        const existing = byPartyId.get(record.partyId);
        byPartyId.set(record.partyId, {
          event: { id: existing?.event.id ?? record.eventId },
          value: {
            partyId: record.partyId,
            clientId: existing?.value.clientId ?? record.clientId,
            displayName: record.displayName,
            status: record.clientStatus ?? existing?.value.status ?? "active",
          },
        });
      }
    }
    return [...byPartyId.values()];
  }, [clientsQuery.data, cofounderOn, customerRecordsQuery.data]);
  const work = workQuery.data;
  const channelNameById = React.useMemo(
    () =>
      new Map(
        clientsQuery.clientChannels.map((channel) => [
          channel.id.toLowerCase(),
          channel.name,
        ]),
      ),
    [clientsQuery.clientChannels],
  );
  const rows = React.useMemo(
    () =>
      records.map(({ event, value }) => ({
        event,
        value,
        work:
          work && value.clientId
            ? sharedWorkForClient(work, value.clientId)
            : null,
      })),
    [records, work],
  );
  const { person } = usePeople(
    rows.flatMap((row) => row.work?.assignedPubkeys ?? []),
  );

  return (
    <ClientWorkspace
      subtitle={
        records.length > 0
          ? `${records.length} ${records.length === 1 ? "client" : "clients"}`
          : undefined
      }
      title="Clients"
    >
      {clientsQuery.channelsQuery.isError ? (
        <RecordMessage kind="error">
          Client channels could not be loaded.
        </RecordMessage>
      ) : null}
      {clientsQuery.liveError ? (
        <RecordMessage kind="error">
          Live client updates could not be started.
        </RecordMessage>
      ) : null}
      {clientsQuery.isError ? (
        <RecordMessage kind="error">
          Client records could not be loaded.
        </RecordMessage>
      ) : null}
      {cofounderOn && customerRecordsQuery.isError ? (
        <RecordMessage kind="error">
          Customer records could not be loaded.
        </RecordMessage>
      ) : null}
      {clientsQuery.isPending ||
      (cofounderOn && customerRecordsQuery.isPending) ? (
        <RecordMessage kind="loading">Loading clients</RecordMessage>
      ) : null}
      {clientsQuery.isSuccess &&
      (!cofounderOn || customerRecordsQuery.isSuccess) &&
      records.length === 0 ? (
        <RecordMessage>
          No client records are available in this business.
        </RecordMessage>
      ) : null}
      {records.length > 0 ? (
        <div className="b3-card b3-table-card">
          <table aria-label="Clients" className="b3-clients">
            <colgroup>
              <col style={{ width: "34.4%" }} />
              <col style={{ width: "calc(166 * var(--b3u))" }} />
              <col />
              <col style={{ width: "calc(58 * var(--b3u))" }} />
            </colgroup>
            <thead>
              <tr>
                <th className="b3-eyebrow" scope="col">
                  Client
                </th>
                <th className="b3-eyebrow" scope="col">
                  Status
                </th>
                <th className="b3-eyebrow" scope="col">
                  Shared work
                </th>
                <th className="b3-sr-only" scope="col">
                  Open
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map(({ event, value, work: sharedWork }) => {
                const owner = sharedWork?.assignedPubkeys[0]
                  ? person(sharedWork.assignedPubkeys[0])
                  : null;
                return (
                  <tr
                    data-testid={`client-row-${value.clientId ?? value.partyId}`}
                    key={event.id}
                    onClick={(clickEvent) => {
                      if (!(clickEvent.target as Element).closest("a")) {
                        if (cofounderOn) {
                          void navigate({
                            to: "/clients/record/$partyId",
                            params: { partyId: value.partyId },
                          });
                        } else if (value.clientId) {
                          void goClient(value.clientId);
                        } else {
                          void navigate({
                            to: "/clients/record/$partyId",
                            params: { partyId: value.partyId },
                          });
                        }
                      }
                    }}
                  >
                    <td>
                      <span className="b3-clients-cell" data-first="true">
                        <span aria-hidden="true" className="b3-tile">
                          {clientInitial(value.displayName)}
                        </span>
                        <span className="b3-stack">
                          {cofounderOn || !value.clientId ? (
                            <Link
                              className="b3-name"
                              params={{ partyId: value.partyId }}
                              to="/clients/record/$partyId"
                            >
                              {value.displayName}
                            </Link>
                          ) : (
                            <Link
                              className="b3-name"
                              params={{ clientId: value.clientId }}
                              to="/clients/$clientId"
                            >
                              {value.displayName}
                            </Link>
                          )}
                          <span className="b3-meta">
                            #
                            {(value.clientId
                              ? channelNameById.get(
                                  value.clientId.toLowerCase(),
                                )
                              : null) ?? value.displayName}
                          </span>
                        </span>
                      </span>
                    </td>
                    <td>
                      <ClientStatusPill size="lg" status={value.status} />
                    </td>
                    <td>
                      <span className="b3-clients-cell">
                        {sharedWork ? (
                          <>
                            {owner ? (
                              <PersonAvatar person={owner} size={32} />
                            ) : null}
                            <span className="b3-stack b3-grow">
                              <span className="b3-work-title">
                                {sharedWork.title}
                              </span>
                              <span className="b3-meta">
                                {owner?.name ?? "Unassigned"}
                              </span>
                            </span>
                            <WorkStatusPill status={sharedWork.status} />
                          </>
                        ) : null}
                      </span>
                    </td>
                    <td aria-hidden="true" className="b3-chevron">
                      <ChevronRightIcon />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ) : null}
      {records.length > 0 ? (
        <div className="b3-note">
          <BriefcaseIcon />
          <span>
            Every client has their own private channel, where their work and
            invoices are shared.
          </span>
        </div>
      ) : null}
    </ClientWorkspace>
  );
}
