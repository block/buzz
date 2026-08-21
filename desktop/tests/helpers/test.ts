import { expect, test as base } from "@playwright/test";

export type * from "@playwright/test";

export { bootstrapE2ePage } from "./bootstrap";

/**
 * Required E2E entrypoint. Register `bootstrapE2ePage(page)` in a `beforeEach`
 * after any bridge, route, or init-script setup and before app interaction.
 */
const test = base;

export { expect, test };
