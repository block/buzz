import { type ReactNode, useCallback, useEffect, useState } from "react";
import { ApiFailure, request } from "./api";
import type { Feedback, Report } from "./types";
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

function Link({ href, children }: { href: string; children: ReactNode }) {
  const { navigate } = usePath();
  return (
    <a
      href={href}
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
    <Page title="Open reports">
      <StateView resource={resource}>
        {(reports) =>
          reports.length ? (
            <div className="cards">
              {reports.map((report) => (
                <Link key={report.id} href={`/reports/${report.id}`}>
                  <article>
                    <header>
                      <strong>{report.reportType}</strong>
                    </header>
                    <p>{report.communityHost}</p>
                    <code>
                      {report.targetKind}: {short(report.target)}
                    </code>
                    <time>{date(report.createdAt)}</time>
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
    <Page title="Report detail">
      <StateView resource={resource}>
        {(report) => (
          <article className="detail">
            <dl>
              <dt>Community</dt>
              <dd>{report.communityHost}</dd>
              <dt>Status</dt>
              <dd>{report.status}</dd>
              <dt>Reporter</dt>
              <dd>
                <code>{report.reporterPubkey}</code>
              </dd>
              <dt>Target</dt>
              <dd>
                <code>{report.target}</code>
              </dd>
              <dt>Note</dt>
              <dd className="sensitive">{report.note ?? "No note"}</dd>
            </dl>
          </article>
        )}
      </StateView>
    </Page>
  );
}

function FeedbackList() {
  const resource = useResource(
    () => request<Feedback[]>("/feedback"),
    "feedback",
  );
  return (
    <Page title="Feedback">
      <StateView resource={resource}>
        {(items) =>
          items.length ? (
            <div className="cards">
              {items.map((item) => (
                <article key={item.id}>
                  <strong>{item.category ?? "uncategorized"}</strong>
                  <p>{item.bodySummary}</p>
                  <code>{short(item.submitterPubkey)}</code>
                  <time>{date(item.receivedAt)}</time>
                </article>
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

function Page({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <header className="page-title">
        <h1>{title}</h1>
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
  return new Date(value).toLocaleString();
}

export function App() {
  const { path } = usePath();
  const report = path.match(/^\/reports\/([^/]+)$/);
  const content = report ? (
    <ReportDetail id={report[1]} />
  ) : path === "/feedback" ? (
    <FeedbackList />
  ) : (
    <Reports />
  );
  return (
    <div className="app">
      <aside>
        <div className="brand">
          Buzz <span>Admin</span>
        </div>
        <nav>
          <Link href="/reports">Reports</Link>
          <Link href="/feedback">Feedback</Link>
        </nav>
        <p className="private">Read-only moderation view</p>
      </aside>
      <main>{content}</main>
    </div>
  );
}
