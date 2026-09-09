# Open-source design system

This implementation is authored for Buzz under the repository Apache-2.0 license. It uses public Base UI behavior primitives, Inter Variable, JetBrains Mono, and Lucide. Their original licenses remain applicable and must accompany redistributed dependencies.

The visual starting point uses neutral surfaces, pill actions, role-based geometry, and clear content hierarchy. No proprietary font files, icon sets, internal screenshots, Figma snapshots, private source files, or internal service clients are included. References informed visual decisions; this is an independently authored implementation, not an export of an internal repository.

## Intentional adaptations

- Inter replaces proprietary sans typography; JetBrains Mono covers code.
- Lucide provides interface icons. Illustrations and examples are original or synthetic.
- Buzz conversation type retains its 13/14/15 preference contract and rem-based zoom.
- Contrast takes priority over exact reference colors.
- The app backdrop and glass remain Buzz materials; dense reading sits on panels.
- Components are now built as a public foundation. Press feedback is tokenized and respects reduced motion.
- CSS and local registries are authoritative. Neither Figma nor any internal service is required to build or run.

## Asset licenses

Inter and JetBrains Mono: SIL Open Font License 1.1. Lucide: ISC, with MIT notices for Feather-derived icons. Base UI: MIT. Preserve the license files distributed with these dependencies. New Buzz source remains Apache-2.0.
