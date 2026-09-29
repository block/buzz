# ORBIT Feature Completion Flow

For every feature:

1. Read the feature specification.
2. Implement the smallest task.
3. Run focused local tests.
4. Continue until the checklist is fully implemented.
5. Run feature integration tests.
6. Run required manual local checks.
7. Run the reviewer checklist.
8. Record test evidence.
9. Mark the feature complete.
10. Update the master feature checklist.
11. Confirm no dependent contract changed unexpectedly.
12. Move to the next feature only after the completion gate passes.

If a test fails, the feature is not complete. Fix the implementation or document an explicit, approved specification decision.
