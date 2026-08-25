# ADR 002: Approval drafts are non-executable

Status: accepted for V6.

V6 lacks a formal approval engine, single-action authorization, real enterprise directory, authority-system write adapters, and a real production test environment. Treating prepared material as approval would create false authority and unsafe automation. Approval Drafts therefore stop at draft preparation and review readiness.

Draft states intentionally exclude approved, rejected, executing, executed, effective, posted, and committed. The UI has no approval or execution controls; attempted authority-changing routes fail closed and are audited. Action and draft types are catalog-controlled, not model-created.

This preserves a complete evidence trail without crossing the authority boundary. Detailed material remains in Business Dock; Buzz receives only identifiers, status, links, and trace IDs. Formal approval and execution require a later, separately authorized stage after real systems and permissions are available.
