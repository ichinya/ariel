# ARIEL development

ARIEL is an independent agent execution service. The core language is Rust;
do not introduce an intermediate Go or Python implementation. Use Tokio for
async work, the official Rust ACP SDK for ACP connections, and SQLite for
durable local state. Specific dependency versions and the SQLite binding
must be selected and verified during the first executable spike.

The service owns sessions, subprocess supervision, events, pending questions
and permission decisions. A local web panel is the first user interface.
External applications are optional clients. Keep their business workflows
outside the core.

## Use Lekalo

Use the Lekalo CLI from the ARIEL repository root. Locate an installed
`lekalo` executable or use `LEKALO_BIN` to select the locally built binary;
do not commit a developer-specific absolute path.

Before work, inspect the current model with `lekalo --no-cache load --ir`
and `lekalo --no-cache doctor`. After model changes, run:

```text
lekalo --no-cache validate
lekalo --no-cache lock --check --offline
```

Maintain semantic definitions in `lekalo/**` when domain contracts are
introduced. Keep `lekalo.lock` with the model. Preview any stale-lock update
with `lekalo update --dry-run --offline` before applying its exact plan.
Do not declare generator, native Rust gate or adapter support without
configured and tested capabilities. Lekalo checks supplement Cargo tests;
they do not prove the Rust implementation works.

## Backlog and evidence

Read the relevant current GitHub issue and comments before implementation.
`planning/backlog/*.json` is a bootstrap snapshot, not the current backlog.
See `planning/rust-core-and-issues.md` for the 2026-09-30 review and proposed
implementation order. Keep planning, mock tests and live runtime evidence
distinct. Preserve unrelated local changes.

The current slice is defined in `planning/pi-mvp.md`: Pi owns its native
tool loop; ARIEL owns provider/model enablement, public API, owned workspaces,
filesystem isolation and explicit grants. Use Pi native RPC first; ACP is
not a dependency of this slice. A worktree or cwd is not a sandbox. Run
`node scripts/check-lekalo.mjs` with `LEKALO_BIN` set after model changes.

## Context7

Use Context7 MCP for current library, framework, SDK, API, CLI and cloud
service documentation, including syntax, setup, configuration, migration
and library-specific debugging. Start with `resolve-library-id` unless an
exact `/org/project` ID is supplied; choose the relevant reputable match
and use `query-docs` with the full question. Use version-specific IDs when
requested. Prefer these docs over web search for library documentation.
This is not required for general programming concepts, business-logic
debugging, code review, refactoring or writing independent scripts.
