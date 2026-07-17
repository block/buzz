import { type ReactNode, useCallback, useEffect, useState } from "react";
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
  return (
    <Page
      eyebrow="Product"
      title="Feedback"
      description="Recent product feedback from across Buzz."
    >
      <StateView resource={resource}>
        {(items) =>
          items.length ? (
            <div className="cards">
              {items.map((item) => (
                <Link
                  key={item.id}
                  href={`/feedback/${item.id}`}
                  className="card-link"
                >
                  <article className="record-card feedback-card">
                    <span className="record-icon feedback-icon">
                      <FeedbackIcon />
                    </span>
                    <div className="record-primary">
                      <span className="tag">
                        {item.category ?? "uncategorized"}
                      </span>
                      <strong>{item.bodySummary}</strong>
                      <span className="record-provenance">
                        {item.communityHost}
                        <code>{short(item.submitterPubkey)}</code>
                      </span>
                    </div>
                    <div className="record-date">
                      <span>Received</span>
                      <time>{date(item.receivedAt)}</time>
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
        {(feedback) => (
          <article className="detail">
            <div className="detail-heading">
              <span className="record-icon feedback-icon">
                <FeedbackIcon />
              </span>
              <div>
                <span className="tag">
                  {feedback.category ?? "uncategorized"}
                </span>
                <h2>{feedback.communityHost}</h2>
              </div>
            </div>
            <dl>
              <dt>Feedback</dt>
              <dd className="sensitive feedback-body">{feedback.body}</dd>
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
        )}
      </StateView>
    </Page>
  );
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
