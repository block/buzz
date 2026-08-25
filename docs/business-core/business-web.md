# Business Web

`apps/business-web` is an independent React/Vite client with full navigation
and compact `/embed` routes for orders, shipments, inventory, receivables and
receipts. Requests use an HttpOnly BusinessSession, CSRF, exact same-origin,
idempotency and expected versions. Tokens/service secrets are never stored in
JavaScript. Business Dock receives only validated resource routes and declared
parent origins.

## Visual regression

The deterministic Playwright suite covers the sales, purchasing, and inventory
pages plus their create dialogs at 80%, 100%, and 125% page zoom. It compares
the page, dialog, and collapsed-navigation screenshots and also asserts that
tables stay inside the page, monetary values remain on one unclipped line,
dialogs remain inside the viewport, and the navigation preference survives a
reload.

```bash
cd apps/business-web
pnpm test:visual
```

Use `pnpm test:visual:update` only after reviewing an intentional visual change.
