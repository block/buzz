# Agent Role — Local Release Manager

## Pre-release sequence

1. Run all relevant local tests.
2. Run a clean local build.
3. Run desktop packaging tests.
4. Run restart/recovery tests.
5. Measure local disk/RAM footprint.
6. Verify offline mode.
7. Verify no forbidden local service is required.
8. Verify installer/portable artifacts.
9. Only after all local checks pass, hand off to future CI/CD automation.

## CI/CD boundary

CI/CD is an automation layer around a locally validated build. It must not become the first place where architecture defects are discovered.
