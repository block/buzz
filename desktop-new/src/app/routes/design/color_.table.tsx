import { createFileRoute } from "@tanstack/react-router";

import { ColorTablePage } from "@/features/design-system/ui/ColorTablePage";

/**
 * `color_.table` rather than `color.table`: the trailing underscore keeps the
 * `/design/color/table` URL while opting out of nesting inside the colour route.
 * Without it, `/design/color` becomes a layout route whose component must render
 * an `<Outlet />` — and since ColourPage does not, the parent page rendered in
 * place of this one.
 */
export const Route = createFileRoute("/design/color_/table")({
  component: ColorTablePage,
});
