# Buzz Wiki — Schema & Conventions

This directory (`wiki/`) is an LLM-maintained knowledge base for the Buzz project.
The LLM creates, updates, and cross-references all pages. Humans read and guide.

## Directory structure

```
wiki/
├── CLAUDE.md           # This file — schema, conventions, workflows
├── index.md            # Catalog of all pages, organized by category
├── log.md              # Append-only chronological record of changes
├── entities/           # Core system entities (Relay, Community, Agent, etc.)
├── concepts/           # Key ideas and mechanisms (Auth, Pipeline, Workflow, etc.)
├── components/         # Crate/service/client reference docs
└── operations/         # Deployment, configuration, development setup
```

## Page conventions

- All pages are Markdown with `.md` extension.
- Every page starts with an `#` title matching the filename (title case).
- Every page should link to related pages using `[display text](../category/pagename)`.
- Pages in `entities/` describe a thing. Pages in `concepts/` describe an idea or mechanism.
- Pages in `components/` document a crate or client. Pages in `operations/` document how-to.
- Keep pages focused. Split into sub-pages when a section gets too long.
- Use `##` for major sections, `###` for subsections.

## Workflows

### Ingest a source
1. Read the source document.
2. Discuss notable points with the user.
3. Create/update relevant entity and concept pages across the wiki.
4. Update `index.md` if new pages were created.
5. Append an entry to `log.md`.

### Answer a query
1. Read `index.md` to find relevant pages.
2. Read the relevant pages.
3. Synthesize an answer with citations to wiki pages.
4. If the answer is valuable as a new page, write it and update `index.md`.
5. Append an entry to `log.md`.

### Lint the wiki
1. Check for contradictions between pages.
2. Find stale claims that newer entries have superseded.
3. Find orphan pages with no inbound links.
4. Find important concepts missing their own page.
5. Suggest new questions and sources to investigate.
6. Append an entry to `log.md`.
