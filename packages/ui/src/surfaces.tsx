import type { ComponentProps, ReactNode } from "react";
import { LoaderCircle } from "lucide-react";
import { cx } from "./classes";
/** A contained group; use rows instead when content forms a list. */
export function Card({ className, ...props }: ComponentProps<"section">) {
  return <section {...props} className={cx("bui-card", className)} />;
}
/** A concise status label; its text carries meaning in addition to color. */
export function Badge({
  tone = "neutral",
  className,
  ...props
}: ComponentProps<"span"> & {
  tone?: "neutral" | "accent" | "success" | "warning" | "danger" | "info";
}) {
  return (
    <span {...props} data-tone={tone} className={cx("bui-badge", className)} />
  );
}
/** An inline notice. Set role=alert only for newly occurring urgent errors. */
export function Alert({
  tone = "info",
  className,
  ...props
}: ComponentProps<"div"> & {
  tone?: "info" | "success" | "warning" | "danger";
}) {
  return (
    <div {...props} data-tone={tone} className={cx("bui-alert", className)} />
  );
}
/** Decorative placeholder; announce loading once on its owning region. */
export function Skeleton({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      {...props}
      aria-hidden="true"
      className={cx("bui-skeleton", className)}
    />
  );
}
/** A named loading status with a decorative animated icon. */
export function Spinner({ label = "Loading" }: { label?: string }) {
  return (
    <span role="status" className="bui-inline">
      <LoaderCircle aria-hidden="true" className="bui-spin" />
      <span>{label}</span>
    </span>
  );
}
/** A stable media frame with a caller-selected ratio. */
export function AspectRatio({
  ratio = 16 / 9,
  style,
  ...props
}: ComponentProps<"div"> & { ratio?: number }) {
  return <div {...props} style={{ ...style, aspectRatio: ratio }} />;
}
/** A semantic table wrapper that allows horizontal scrolling on narrow screens. */
export function Table({ children, ...props }: ComponentProps<"table">) {
  return (
    <div className="bui-table-scroll">
      <table {...props} className={cx("bui-table", props.className)}>
        {children}
      </table>
    </div>
  );
}
/** A quiet division between related regions. */
export function Separator(props: ComponentProps<"hr">) {
  return <hr {...props} className={cx("bui-separator", props.className)} />;
}
/** Empty states explain purpose and provide an explicit next action. */
export function EmptyState({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="bui-empty">
      <h3 className="bui-label">{title}</h3>
      <p className="bui-description">{description}</p>
      {action}
    </div>
  );
}
/** Related destinations with one named navigation landmark. */
export function Breadcrumb({ children, ...props }: ComponentProps<"nav">) {
  return (
    <nav aria-label="Breadcrumb" {...props}>
      <ol className="bui-breadcrumb">{children}</ol>
    </nav>
  );
}
/** A side navigation region; selection is represented by aria-current on links. */
export function Sidebar(
  props: ComponentProps<"nav"> & { "aria-label": string },
) {
  return <nav {...props} className={cx("bui-sidebar", props.className)} />;
}
/** Page navigation; callers own links and mark the current one with aria-current=page. */
export function Pagination(props: ComponentProps<"nav">) {
  return (
    <nav
      aria-label="Pagination"
      {...props}
      className={cx("bui-toolbar", props.className)}
    />
  );
}
