# ADR 001: Human-confirmed work items

Status: accepted for V6.

Agent output and natural language are not sufficient authority to create durable workflow state. Agent text may be ambiguous, injected, or detached from the user's active BusinessSession and current scope. Therefore Action Codes come only from the versioned catalog, and Work Item creation requires a short-lived hash-bound preview plus an explicit click in Business Dock.

The Dock is the confirmation boundary because it can enforce the active business identity, CSRF, Origin, capability, scope, assignee, and current finding version. Likes, emoji, channel membership, Buzz events, and Agent tool intent are not confirmation.

A Work Item means only “a human-confirmed internal follow-up exists.” It is not a shipment hold, purchase cancellation, inventory adjustment, payment, journal entry, or any other authority-system action. Finding condition and human review status remain separate so analytical truth is not overwritten by workflow opinion. Buzz stores only a compact summary and link so detailed and sensitive workflow data stays in the Business System.
