# Business Action V6

V6 turns deterministic anomaly findings into server-controlled action proposals, human-confirmed internal work items, and non-executable approval drafts. It never updates sales, purchase, inventory, receivable, payable, payment, invoice, journal, tax, credit, or shipment-control authority data.

The current implementation is **Desensitized Acceptance — Production Disabled**. Acceptance users, scopes, findings, and UI fixtures are synthetic or desensitized. Production mode is the release default and refuses to start until formal authorization and enterprise-directory adapters exist.

Read [architecture](architecture.md), [finding lifecycle](finding-lifecycle.md), [catalog](action-catalog.md), [proposals](action-proposal.md), [work items](work-items.md), [approval drafts](approval-drafts.md), [authorization](authorization.md), [audit](audit.md), [Dock integration](business-dock-integration.md), and [operations](operations.md).
