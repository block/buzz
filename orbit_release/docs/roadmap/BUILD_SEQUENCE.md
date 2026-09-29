# ORBIT Build Sequence

## Local V1

F01 -> F02 -> F03 -> F04 -> F05 -> F06 -> F07 -> F08 -> F09 -> F10

The dependency graph in `features/FEATURE_ORDER.md` is authoritative.

## Hosted / enterprise after local validation

F11 -> F12 -> F13

F11 establishes the hosted data plane and parity contracts. F12 adds accounts, entitlements, device identity and logical sync. F13 adds the dedicated auth server and the enterprise policy/governance enforcement layer.

## Do not reverse the order

Do not build cloud-first storage and retrofit the desktop later. Local V1 must establish the logical memory schema, evidence/provenance model, retrieval contracts, and rebuildable local indexes first.
