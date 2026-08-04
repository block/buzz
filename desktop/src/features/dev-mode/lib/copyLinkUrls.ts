/**
 * Transcript links render a fetched title ("joah in #general") instead of
 * their URL, so a plain copy would lose the destination. This handler
 * rewrites the plain-text clipboard so every DevLink serializes as its
 * href, while text/html keeps the pretty labels for rich paste targets.
 *
 * Returns true when it handled the event (selection contained a DevLink).
 */
export function copySelectionWithLinkUrls(event: ClipboardEvent): boolean {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) {
    return false;
  }

  const container = document.createElement("div");
  for (let i = 0; i < selection.rangeCount; i += 1) {
    container.appendChild(selection.getRangeAt(i).cloneContents());
  }
  if (!container.querySelector("a[data-dev-link]")) {
    return false;
  }

  const html = container.innerHTML;

  for (const anchor of container.querySelectorAll("a[data-dev-link]")) {
    const href = anchor.getAttribute("href");
    if (href) anchor.replaceWith(document.createTextNode(href));
  }

  // innerText needs layout to turn block boundaries into newlines, so the
  // clone briefly joins the document off-screen.
  container.style.position = "fixed";
  container.style.left = "-99999px";
  container.style.top = "0";
  document.body.appendChild(container);
  const text = container.innerText;
  container.remove();

  event.clipboardData?.setData("text/plain", text);
  event.clipboardData?.setData("text/html", html);
  event.preventDefault();
  return true;
}
