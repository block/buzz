import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * True when following `href` leaves this site. Same-origin paths, in-page
 * anchors, and custom schemes (`buzz://`, `mailto:`) stay in the current tab —
 * a new tab for those would just leave an empty one behind.
 */
export function isOffSiteHref(href: string | undefined): boolean {
  if (!href) return false;
  try {
    const url = new URL(href, window.location.href);
    return (
      /^https?:$/.test(url.protocol) && url.origin !== window.location.origin
    );
  } catch {
    return false;
  }
}

/** Markdown (GitHub flavour) that opens off-site links in a new tab. */
export function Markdown({ children }: { children: string }) {
  return (
    <ReactMarkdown
      components={{
        a({ node: _node, href, ...props }) {
          return isOffSiteHref(href) ? (
            <a
              href={href}
              rel="noopener noreferrer"
              target="_blank"
              {...props}
            />
          ) : (
            <a href={href} {...props} />
          );
        },
      }}
      remarkPlugins={[remarkGfm]}
    >
      {children}
    </ReactMarkdown>
  );
}
