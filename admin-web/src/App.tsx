import {
  type ChangeEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import { ApiFailure, request } from "./api";
import type { FeedbackDetail, FeedbackSummary, Report } from "./types";
import { useResource } from "./useResource";

function usePath() {
  const [path, setPath] = useState(location.pathname);
  useEffect(() => {
    const update = () => setPath(location.pathname);
    addEventListener("popstate", update);
    return () => removeEventListener("popstate", update);
  }, []);
  const navigate = useCallback((url: string) => {
    history.pushState(null, "", url);
    dispatchEvent(new PopStateEvent("popstate"));
  }, []);
  return { path, navigate };
}

function Link({
  href,
  className,
  activeWhenNested = false,
  children,
}: {
  href: string;
  className?: string;
  activeWhenNested?: boolean;
  children: ReactNode;
}) {
  const { path, navigate } = usePath();
  const active =
    path === href || (activeWhenNested && path.startsWith(`${href}/`));
  return (
    <a
      href={href}
      className={className}
      aria-current={active ? "page" : undefined}
      onClick={(event) => {
        if (!event.metaKey && !event.ctrlKey) {
          event.preventDefault();
          navigate(href);
        }
      }}
    >
      {children}
    </a>
  );
}

function StateView<T>({
  resource,
  children,
}: {
  resource: ReturnType<typeof useResource<T>>;
  children: (data: T) => ReactNode;
}) {
  if (resource.loading && !resource.data)
    return <div className="state">Loading…</div>;
  if (resource.error && !resource.data) {
    const forbidden =
      resource.error instanceof ApiFailure && resource.error.status === 403;
    return (
      <div className="state error" role="alert">
        <h2>{forbidden ? "Access denied" : "Could not load data"}</h2>
        <p>{resource.error.message}</p>
        <button type="button" onClick={resource.refetch}>
          Retry
        </button>
      </div>
    );
  }
  return resource.data ? children(resource.data) : null;
}

function Reports() {
  const resource = useResource(
    () => request<Report[]>("/reports?status=open&limit=100"),
    "reports",
  );
  return (
    <Page
      eyebrow="Moderation"
      title="Open reports"
      description="Review reports across every Buzz community."
    >
      <StateView resource={resource}>
        {(reports) =>
          reports.length ? (
            <div className="cards">
              {reports.map((report) => (
                <Link
                  key={report.id}
                  href={`/reports/${report.id}`}
                  className="card-link"
                >
                  <article className="record-card">
                    <span className="record-icon report-icon">
                      <ReportIcon />
                    </span>
                    <div className="record-primary">
                      <span className="tag">{report.reportType}</span>
                      <strong>{report.communityHost}</strong>
                      <code>
                        {report.targetKind}: {short(report.target)}
                      </code>
                    </div>
                    <div className="record-date">
                      <span>Submitted</span>
                      <time>{date(report.createdAt)}</time>
                    </div>
                    <ArrowIcon />
                  </article>
                </Link>
              ))}
            </div>
          ) : (
            <Empty />
          )
        }
      </StateView>
    </Page>
  );
}

function ReportDetail({ id }: { id: string }) {
  const resource = useResource(() => request<Report>(`/reports/${id}`), id);
  return (
    <Page
      eyebrow="Moderation"
      title="Report detail"
      description="The full report as submitted to the relay."
      back="/reports"
    >
      <StateView resource={resource}>
        {(report) => (
          <article className="detail">
            <div className="detail-heading">
              <span className="record-icon report-icon">
                <ReportIcon />
              </span>
              <div>
                <span className="tag">{report.reportType}</span>
                <h2>{report.communityHost}</h2>
              </div>
            </div>
            <dl>
              <dt>Status</dt>
              <dd>
                <span className="status">{report.status}</span>
              </dd>
              <dt>Reporter</dt>
              <dd>
                <code>{report.reporterPubkey}</code>
              </dd>
              <dt>Target</dt>
              <dd>
                <code>{report.target}</code>
              </dd>
              <dt>Note</dt>
              <dd className="sensitive">
                {report.note ?? "No note provided."}
              </dd>
            </dl>
          </article>
        )}
      </StateView>
    </Page>
  );
}

function FeedbackList() {
  const resource = useResource(
    () => request<FeedbackSummary[]>("/feedback"),
    "feedback",
  );
  const [query, setQuery] = useState("");
  const [community, setCommunity] = useState("all");
  const [timeRange, setTimeRange] = useState("all");
  const [statusFilter, setStatusFilter] = useState("all");
  const [statuses, setStatuses] = useState(loadFeedbackStatuses);

  const updateStatus = (id: string, event: ChangeEvent<HTMLInputElement>) => {
    const checked = event.target.checked;
    setStatuses((current) => {
      const next = {
        ...current,
        [id]: checked,
      };
      saveFeedbackStatuses(next);
      return next;
    });
  };

  return (
    <Page
      eyebrow="Product"
      title="Feedback"
      description="Recent product feedback from across Buzz."
    >
      <StateView resource={resource}>
        {(items) => {
          if (!items.length) return <Empty />;
          return (
            <FeedbackResults
              items={items}
              query={query}
              community={community}
              timeRange={timeRange}
              statusFilter={statusFilter}
              statuses={statuses}
            >
              {({ communities, filtered }) => (
                <>
                  <div className="feedback-filters">
                    <label className="search-field">
                      <span>Search feedback</span>
                      <div>
                        <SearchIcon />
                        <input
                          type="search"
                          placeholder="Search feedback"
                          value={query}
                          onChange={(event) => setQuery(event.target.value)}
                        />
                      </div>
                    </label>
                    <label>
                      <span>Community</span>
                      <select
                        value={community}
                        onChange={(event) => setCommunity(event.target.value)}
                      >
                        <option value="all">All communities</option>
                        {communities.map((host) => (
                          <option key={host} value={host}>
                            {host}
                          </option>
                        ))}
                      </select>
                    </label>
                    <label>
                      <span>Received</span>
                      <select
                        value={timeRange}
                        onChange={(event) => setTimeRange(event.target.value)}
                      >
                        <option value="all">Any time</option>
                        <option value="day">Last 24 hours</option>
                        <option value="week">Last 7 days</option>
                        <option value="month">Last 30 days</option>
                      </select>
                    </label>
                    <label>
                      <span>Status</span>
                      <select
                        value={statusFilter}
                        onChange={(event) =>
                          setStatusFilter(event.target.value)
                        }
                      >
                        <option value="all">Any status</option>
                        <option value="pending">Needs action</option>
                        <option value="acted-on">Acted on</option>
                      </select>
                    </label>
                  </div>
                  <p className="result-count" aria-live="polite">
                    {filtered.length} of {items.length} submissions
                  </p>
                  {filtered.length ? (
                    <div className="cards">
                      {filtered.map((item) => (
                        <article
                          key={item.id}
                          className="record-card feedback-card feedback-record"
                        >
                          <Link
                            href={`/feedback/${item.id}`}
                            className="feedback-main-link"
                          >
                            <span className="record-icon feedback-icon">
                              <CategoryIcon category={item.category} />
                            </span>
                            <div className="record-primary">
                              <CategoryTag category={item.category} />
                              <strong>{item.bodySummary}</strong>
                              <span className="record-provenance">
                                {item.communityHost}
                                <code>{short(item.submitterPubkey)}</code>
                              </span>
                            </div>
                          </Link>
                          <label className="feedback-status">
                            <input
                              type="checkbox"
                              checked={statuses[item.id] ?? false}
                              onChange={(event) => updateStatus(item.id, event)}
                            />
                            Acted on
                            <span className="visually-hidden">
                              feedback from {item.communityHost}
                            </span>
                          </label>
                          <div className="record-date">
                            <span>Received</span>
                            <time>{date(item.receivedAt)}</time>
                          </div>
                          <Link
                            href={`/feedback/${item.id}`}
                            className="record-open-link"
                          >
                            <span className="visually-hidden">
                              Open feedback from {item.communityHost}
                            </span>
                            <ArrowIcon />
                          </Link>
                        </article>
                      ))}
                    </div>
                  ) : (
                    <div className="state">No matching feedback.</div>
                  )}
                </>
              )}
            </FeedbackResults>
          );
        }}
      </StateView>
    </Page>
  );
}

function FeedbackResults({
  items,
  query,
  community,
  timeRange,
  statusFilter,
  statuses,
  children,
}: {
  items: FeedbackSummary[];
  query: string;
  community: string;
  timeRange: string;
  statusFilter: string;
  statuses: FeedbackStatuses;
  children: (results: {
    communities: string[];
    filtered: FeedbackSummary[];
  }) => ReactNode;
}) {
  const results = useMemo(() => {
    const communities = [...new Set(items.map((item) => item.communityHost))]
      .filter(Boolean)
      .sort((left, right) => left.localeCompare(right));
    const normalizedQuery = query.trim().toLocaleLowerCase();
    const after = timeRangeStart(timeRange);
    const filtered = items.filter((item) => {
      if (community !== "all" && item.communityHost !== community) return false;
      if (statusFilter === "pending" && statuses[item.id]) return false;
      if (statusFilter === "acted-on" && !statuses[item.id]) return false;
      if (after !== undefined) {
        const receivedAt = new Date(item.receivedAt).valueOf();
        if (Number.isNaN(receivedAt) || receivedAt < after) return false;
      }
      if (!normalizedQuery) return true;
      return [
        item.bodySummary,
        item.communityHost,
        item.category ?? "uncategorized",
        item.submitterPubkey,
      ].some((value) => value.toLocaleLowerCase().includes(normalizedQuery));
    });
    return { communities, filtered };
  }, [items, query, community, timeRange, statusFilter, statuses]);
  return children(results);
}

function FeedbackDetailView({ id }: { id: string }) {
  const resource = useResource(
    () => request<FeedbackDetail>(`/feedback/${id}`),
    id,
  );
  return (
    <Page
      eyebrow="Product"
      title="Feedback detail"
      description="The complete feedback submission and its source."
      back="/feedback"
      backLabel="Back to feedback"
    >
      <StateView resource={resource}>
        {(feedback) => {
          const attachments = feedbackAttachments(
            feedback.id,
            feedback.tags,
            feedback.communityHost,
          );
          const body = stripAttachmentMarkdown(feedback.body, attachments);
          return (
            <article className="detail">
              <div className="detail-heading">
                <span className="record-icon feedback-icon">
                  <CategoryIcon category={feedback.category} />
                </span>
                <div>
                  <CategoryTag category={feedback.category} />
                  <h2>{feedback.communityHost}</h2>
                </div>
              </div>
              <dl>
                <dt>Feedback</dt>
                <dd className="sensitive feedback-body">{body}</dd>
                {attachments.length ? (
                  <>
                    <dt>Attachments</dt>
                    <dd className="attachments">
                      {attachments.map((attachment) => (
                        <Attachment
                          key={`${attachment.hash}-${attachment.url}`}
                          attachment={attachment}
                        />
                      ))}
                    </dd>
                  </>
                ) : null}
                <dt>Submitted by</dt>
                <dd>
                  <code>{feedback.submitterPubkey}</code>
                </dd>
                <dt>Event</dt>
                <dd>
                  <code>{feedback.eventId}</code>
                </dd>
                <dt>Created</dt>
                <dd>{date(feedback.eventCreatedAt)}</dd>
                <dt>Received</dt>
                <dd>{date(feedback.receivedAt)}</dd>
              </dl>
            </article>
          );
        }}
      </StateView>
    </Page>
  );
}

type FeedbackStatuses = Record<string, boolean>;

interface FeedbackAttachment {
  url: string;
  sourceUrl: string;
  mimeType: string;
  hash: string;
  size?: number;
  dimensions?: string;
  filename?: string;
}

const FEEDBACK_STATUS_KEY = "buzz-admin-feedback-status";

function loadFeedbackStatuses(): FeedbackStatuses {
  try {
    const stored = localStorage.getItem(FEEDBACK_STATUS_KEY);
    return stored ? (JSON.parse(stored) as FeedbackStatuses) : {};
  } catch {
    return {};
  }
}

function saveFeedbackStatuses(statuses: FeedbackStatuses) {
  try {
    localStorage.setItem(FEEDBACK_STATUS_KEY, JSON.stringify(statuses));
  } catch {
    // The controls remain useful for the current session if storage is blocked.
  }
}

function timeRangeStart(range: string) {
  const durations: Record<string, number> = {
    day: 24 * 60 * 60 * 1000,
    week: 7 * 24 * 60 * 60 * 1000,
    month: 30 * 24 * 60 * 60 * 1000,
  };
  const duration = durations[range];
  return duration ? Date.now() - duration : undefined;
}

function feedbackAttachments(
  feedbackId: string,
  tags: string[][],
  communityHost: string,
): FeedbackAttachment[] {
  return tags.flatMap((tag) => {
    if (tag[0] !== "imeta") return [];
    const values = new Map<string, string>();
    for (const entry of tag.slice(1)) {
      const separator = entry.indexOf(" ");
      if (separator > 0) {
        values.set(entry.slice(0, separator), entry.slice(separator + 1));
      }
    }
    const url = values.get("url");
    const mimeType = values.get("m");
    const hash = values.get("x");
    const safeUrl = url && safeAttachmentUrl(url, communityHost);
    if (!safeUrl || !mimeType || !hash) return [];
    const parsedSize = Number(values.get("size"));
    return [
      {
        url: `/api/admin/v1/feedback/${encodeURIComponent(feedbackId)}/attachments/${hash}`,
        sourceUrl: safeUrl,
        mimeType,
        hash,
        size:
          Number.isFinite(parsedSize) && parsedSize > 0
            ? parsedSize
            : undefined,
        dimensions: values.get("dim"),
        filename: values.get("filename"),
      },
    ];
  });
}

function safeAttachmentUrl(value: string, communityHost: string) {
  try {
    const url = new URL(value, `${location.protocol}//${communityHost}`);
    return ["http:", "https:"].includes(url.protocol) &&
      url.host.toLocaleLowerCase() === communityHost.toLocaleLowerCase() &&
      url.pathname.startsWith("/media/")
      ? url.href
      : undefined;
  } catch {
    return undefined;
  }
}

function stripAttachmentMarkdown(
  body: string,
  attachments?: FeedbackAttachment[],
) {
  const knownUrls = attachments
    ? new Set(attachments.map((attachment) => attachment.sourceUrl))
    : undefined;
  return body
    .replace(/!?\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g, (match, url) => {
      const isMedia = knownUrls ? knownUrls.has(url) : url.includes("/media/");
      return isMedia ? "" : match;
    })
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function Attachment({ attachment }: { attachment: FeedbackAttachment }) {
  const url = attachment.url;
  const name =
    attachment.filename ?? `attachment-${attachment.hash.slice(0, 8)}`;
  const metadata = [
    attachment.mimeType,
    attachment.dimensions,
    attachment.size ? formatBytes(attachment.size) : undefined,
  ]
    .filter(Boolean)
    .join(" · ");

  if (attachment.mimeType.startsWith("image/")) {
    return (
      <figure className="image-attachment">
        <a href={url} target="_blank" rel="noreferrer">
          <img src={url} alt={name} loading="lazy" />
        </a>
        <figcaption>
          <span>{name}</span>
          <small>{metadata}</small>
        </figcaption>
      </figure>
    );
  }

  return (
    <a
      className="file-attachment"
      href={url}
      target="_blank"
      rel="noreferrer"
      download={name}
    >
      <FileIcon />
      <span>
        <strong>{name}</strong>
        <small>{metadata}</small>
      </span>
      <ArrowIcon />
    </a>
  );
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function Page({
  eyebrow,
  title,
  description,
  back,
  backLabel,
  children,
}: {
  eyebrow: string;
  title: string;
  description: string;
  back?: string;
  backLabel?: string;
  children: ReactNode;
}) {
  return (
    <section>
      <header className="page-title">
        {back ? (
          <Link href={back} className="back-link">
            <ArrowIcon /> {backLabel ?? "Back to reports"}
          </Link>
        ) : null}
        <p>{eyebrow}</p>
        <h1>{title}</h1>
        <span>{description}</span>
      </header>
      {children}
    </section>
  );
}

function Empty() {
  return <div className="state">No records.</div>;
}

function short(value: string) {
  return value.length > 20 ? `${value.slice(0, 10)}…${value.slice(-8)}` : value;
}

function date(value: string) {
  const parsed = new Date(value);
  return Number.isNaN(parsed.valueOf())
    ? "Unknown date"
    : parsed.toLocaleString();
}

function BuzzMark() {
  return (
    <svg viewBox="0 0 466 309" aria-hidden="true">
      <path d="M91.7 62.8a91.7 91.7 0 0 0 0 183.4H128V62.8H91.7Zm282.6 0H338v183.4h36.3a91.7 91.7 0 1 0 0-183.4Z" />
      <path
        fillRule="evenodd"
        d="M162 0h142a34 34 0 0 1 34 34v241a34 34 0 0 1-34 34H162a34 34 0 0 1-34-34V34a34 34 0 0 1 34-34Zm31.3 57.4a27 27 0 1 0 0 54 27 27 0 0 0 0-54Zm82.7 0a27 27 0 1 0 0 54 27 27 0 0 0 0-54Zm-109.7 99.8h136.9v38.3H166.3v-38.3Zm.6 77.9h136.2v37.6H166.9v-37.6Z"
        clipRule="evenodd"
      />
    </svg>
  );
}

function ReportIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 3 4.5 6v5.2c0 4.7 3.2 8.8 7.5 9.8 4.3-1 7.5-5.1 7.5-9.8V6L12 3Z" />
      <path d="M12 7.5v5M12 16.5h.01" />
    </svg>
  );
}

function FeedbackIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M5 5.5h14v10H9l-4 3v-13Z" />
      <path d="M8.5 9h7M8.5 12h4.5" />
    </svg>
  );
}

function CategoryTag({ category }: { category?: string }) {
  const labels: Record<string, string> = {
    bug: "Bug",
    praise: "Praise",
    "needs-work": "Needs work",
  };
  return (
    <span className="tag">
      <CategoryIcon category={category} />
      {category ? (labels[category] ?? category) : "Uncategorized"}
    </span>
  );
}

function CategoryIcon({ category }: { category?: string }) {
  if (category === "bug") return <BugIcon />;
  if (category === "praise") return <ThumbsUpIcon />;
  if (category === "needs-work") return <WrenchIcon />;
  return <FeedbackIcon />;
}

function BugIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 20v-9" />
      <path d="M14 7a4 4 0 0 1 4 4v3a6 6 0 0 1-12 0v-3a4 4 0 0 1 4-4z" />
      <path d="M14.12 3.88 16 2M21 21a4 4 0 0 0-3.81-4M21 5a4 4 0 0 1-3.55 3.97M22 13h-4M3 21a4 4 0 0 1 3.81-4M3 5a4 4 0 0 0 3.55 3.97M6 13H2M8 2l1.88 1.88M9 7.13V6a3 3 0 1 1 6 0v1.13" />
    </svg>
  );
}

function ThumbsUpIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M15 5.88 14 10h5.83a2 2 0 0 1 1.92 2.56l-2.33 8A2 2 0 0 1 17.5 22H4a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2h2.76a2 2 0 0 0 1.79-1.11L12 2a3.13 3.13 0 0 1 3 3.88Z" />
      <path d="M7 10v12" />
    </svg>
  );
}

function WrenchIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.106-3.105c.32-.322.863-.22.983.218a6 6 0 0 1-8.259 7.057l-7.91 7.91a1 1 0 0 1-2.999-3l7.91-7.91a6 6 0 0 1 7.057-8.259c.438.12.54.662.219.984z" />
    </svg>
  );
}

function SearchIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="11" cy="11" r="6.5" />
      <path d="m16 16 4 4" />
    </svg>
  );
}

function FileIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M6 3h8l4 4v14H6V3Z" />
      <path d="M14 3v5h4M9 13h6M9 17h4" />
    </svg>
  );
}

function ArrowIcon() {
  return (
    <svg className="arrow-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="m9 18 6-6-6-6" />
    </svg>
  );
}

export function App() {
  const { path } = usePath();
  const report = path.match(/^\/reports\/([^/]+)$/);
  const feedback = path.match(/^\/feedback\/([^/]+)$/);
  const content = report ? (
    <ReportDetail id={report[1]} />
  ) : feedback ? (
    <FeedbackDetailView id={feedback[1]} />
  ) : path === "/feedback" ? (
    <FeedbackList />
  ) : (
    <Reports />
  );
  return (
    <div className="app">
      <header className="app-header">
        <Link href="/reports" className="brand">
          <span className="brand-mark">
            <BuzzMark />
          </span>
          <span>
            Buzz <b>Admin</b>
          </span>
        </Link>
        <nav>
          <Link href="/reports" className="nav-link" activeWhenNested>
            <ReportIcon /> Reports
          </Link>
          <Link href="/feedback" className="nav-link" activeWhenNested>
            <FeedbackIcon /> Feedback
          </Link>
        </nav>
      </header>
      <main>{content}</main>
    </div>
  );
}
