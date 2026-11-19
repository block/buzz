const OPEN_PREFIX = "/open/";

type NavigationForm = {
  /** `none`: nothing after the host; `ids`: one or two id segments. */
  path: "none" | "ids";
  /** Query keys that must appear exactly once. */
  required: readonly string[];
  /** Query keys that may appear at most once. */
  optional: readonly string[];
};

/**
 * `buzz://` navigation hosts and the fields each needs. Each opens a
 * destination in the selected community. Keep in sync with `parseBuzzLink` in
 * block/buzz-app src/features/navigation/buzz-links.ts and the navigation arms
 * of `handle_deep_link_url` in desktop/src-tauri/src/deep_link.rs.
 */
const NAVIGATION_HOSTS = new Map<string, NavigationForm>([
  [
    "message",
    { path: "none", required: ["channel", "id"], optional: ["thread"] },
  ],
  ["channel", { path: "ids", required: [], optional: [] }],
  [
    "repo",
    { path: "none", required: ["owner", "d"], optional: ["tab", "commit"] },
  ],
  ["project", { path: "none", required: [], optional: [] }],
  ["pr", { path: "none", required: ["owner", "d", "id"], optional: [] }],
  ["issue", { path: "none", required: ["owner", "d", "id"], optional: [] }],
]);

function pathFits(form: NavigationForm, segments: string[]) {
  if (form.path === "none")
    return segments.length === 0 || (segments.length === 1 && !segments[0]);
  return (
    segments.length >= 1 &&
    segments.length <= 2 &&
    segments.every((segment) => segment !== "")
  );
}

function queryFits(form: NavigationForm, search: string) {
  const params = new URLSearchParams(search);
  const present = (key: string) => params.getAll(key).every((value) => value);
  return (
    form.required.every(
      (key) => params.getAll(key).length === 1 && present(key),
    ) &&
    form.optional.every((key) => params.getAll(key).length <= 1 && present(key))
  );
}

/**
 * Maps `/open/<rest>` to `buzz://<rest>`, keeping path and query byte for
 * byte. Anything but a navigation link with its required fields yields
 * `null`, so this public page never forwards invite, authorization or install
 * links, nor links the app would ignore. Unknown query keys pass through, as
 * the desktop app ignores them.
 */
export function buzzLinkFromOpenUrl(
  pathname: string,
  search: string,
): string | null {
  if (!pathname.startsWith(OPEN_PREFIX)) return null;
  const rest = pathname.slice(OPEN_PREFIX.length);
  const [host = "", ...segments] = rest.split("/");
  const form = NAVIGATION_HOSTS.get(host);
  if (!form || !pathFits(form, segments) || !queryFits(form, search))
    return null;
  return `buzz://${rest}${search}`;
}
