import { Link, Outlet } from "@tanstack/react-router";
import { Fragment, type ReactNode } from "react";

import { useColorScheme } from "@/shared/theme/useColorScheme";
import { COMPONENTS } from "@/shared/ui/registry";

/** A nav entry, optionally with children shown indented beneath it. */
type NavItem = [label: string, to: string, children?: NavItem[]];

function componentNavItems(parent?: string): NavItem[] {
  return COMPONENTS.filter((component) => component.parent === parent).map(
    (component) => [
      component.name,
      `/design/components/${component.slug}`,
      componentNavItems(component.slug),
    ],
  );
}

const COMPONENT_NAV_ITEMS: NavItem[] = [
  ["Overview", "/design/components", componentNavItems()],
  ["Base UI backing", "/design/components/base-ui"],
];

const SECTIONS: Array<{ heading: string; items: NavItem[] }> = [
  {
    heading: "Foundations",
    items: [
      // "Color" in the interface. The code still says `Colour` in places —
      // registry types, the ColourPage component — and renaming those is a
      // separate change from what the nav says.
      ["Color", "/design/color", [["Token table", "/design/color/table"]]],
      ["Typography", "/design/typography"],
      ["Spacing", "/design/spacing"],
      ["Radius", "/design/radius"],
      ["Elevation", "/design/elevation"],
      ["Glass", "/design/glass"],
      ["Motion", "/design/motion"],
    ],
  },
  {
    heading: "Product compositions",
    items: [["Composer", "/design/composer"]],
  },
  {
    heading: "System",
    items: [
      ["Vocabulary", "/design/vocabulary"],
      ["Growing the system", "/design/growth"],
    ],
  },
  {
    heading: "Components",
    items: COMPONENT_NAV_ITEMS,
  },
];

function NavLink({
  to,
  exact,
  children,
}: {
  to: string;
  /** Set on a parent that has children, so it does not stay lit on a subpage. */
  exact?: boolean;
  children: ReactNode;
}) {
  return (
    <Link
      to={to}
      activeOptions={exact ? { exact: true } : undefined}
      className="block rounded-lg px-3 py-2 text-body text-secondary transition-colors hover:bg-neutral-4 hover:text-primary"
      activeProps={{
        className:
          "block rounded-lg px-3 py-2 text-body bg-purple-3 text-purple-12",
      }}
    >
      {children}
    </Link>
  );
}

function NavItems({ items, depth = 0 }: { items: NavItem[]; depth?: number }) {
  return (
    <div className="flex flex-wrap gap-1 lg:flex-col">
      {items.map(([label, to, children]) => (
        <Fragment key={to}>
          <div className={depth === 0 ? undefined : "lg:pl-3"}>
            <NavLink
              to={to}
              exact={children !== undefined && children.length > 0}
            >
              {label}
            </NavLink>
          </div>
          {children && children.length > 0 ? (
            <NavItems items={children} depth={depth + 1} />
          ) : null}
        </Fragment>
      ))}
    </div>
  );
}

export function DesignSystemLayout() {
  const { scheme, toggle } = useColorScheme();

  return (
    /* Narrow: the nav stacks above the content as a wrapped list, because a
       256px column beside a reading column leaves neither enough room. From lg
       it becomes the sticky side rail. */
    <div className="flex min-h-screen flex-col bg-panel lg:flex-row">
      <nav
        aria-label="Design system"
        className="flex shrink-0 flex-col gap-8 px-4 py-8 lg:sticky lg:top-0 lg:h-screen lg:w-64 lg:overflow-y-auto"
      >
        <div className="px-3">
          <Link to="/design" className="text-body text-primary">
            Buzz Design System
          </Link>
          <p className="mt-1 text-body-sm text-tertiary">
            Rendered from the tokens themselves
          </p>
        </div>

        <div className="flex flex-1 flex-col gap-6 lg:gap-7">
          {SECTIONS.map((section) => (
            <div key={section.heading} className="flex flex-col gap-1">
              <h2 className="px-3 pb-1.5 text-body-sm text-tertiary">
                {section.heading}
              </h2>
              {section.items.length === 0 ? (
                <p className="max-w-prose px-3 py-1 text-body-sm text-tertiary">
                  None yet — the primitive layer gets built one component at a
                  time, as the product repeats something.
                </p>
              ) : (
                <NavItems items={section.items} />
              )}
            </div>
          ))}
        </div>

        <button
          type="button"
          onClick={toggle}
          aria-label={`Switch to ${scheme === "light" ? "dark" : "light"} mode`}
          className="mx-3 self-start rounded-lg bg-neutral-2 px-3 py-2 text-body text-secondary transition-colors hover:bg-neutral-4 hover:text-primary"
        >
          {scheme === "light" ? "Dark mode" : "Light mode"}
        </button>
      </nav>

      <main className="min-w-0 flex-1 px-6 py-10 sm:px-10 lg:px-16">
        <div className="mx-auto max-w-3xl">
          <Outlet />
        </div>
      </main>
    </div>
  );
}
