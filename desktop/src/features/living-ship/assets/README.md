# Living Ship visual assets

Both assets were generated with the built-in image generation tool on 2 August 2026 and inspected at original resolution before use.

## `hmas-supply-living-ship.png`

Reference: the supplied HMAS Stalwart/Supply side-elevation drawing. The prompt required a strict broadside 16-bit pixel-art auxiliary ship with the complete hull, flight deck, masts, replenishment rigs, bridge and bow silhouette. It also required exactly two stacked aft rooms and a forward two-column by three-row compartment grid, with no labels or people. The selected output is retained at its native 1754×896 aspect ratio so the complete mast-to-waterline silhouette remains visible without crop padding or geometric distortion.

Generated source: `/Users/matthewwarren/.codex/generated_images/019fc003-63db-7f21-a794-d93de1e7d7e2/exec-27ba0ed3-9c26-4bb9-922e-e1f14e6712d1.png`

## `agent-sprites.png`

The prompt required exactly eight matching 16-bit naval staff figures in command-team order: Chief of Staff, Operations, Maritime N2, Logistics, Navigation, Daily Routine, Reporting and Plans. The generated sheet used a flat green chroma background, which was removed with the image-generation skill's `remove_chroma_key.py` helper. The final 2172×724 PNG has an alpha channel and no embedded labels.

Generated source: `/Users/matthewwarren/.codex/generated_images/019fc003-63db-7f21-a794-d93de1e7d7e2/exec-7ed23d00-e45c-4366-94c8-d051d44b4a65.png`
