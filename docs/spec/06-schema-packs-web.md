# Phase 6: schema v1, packs, Issues and Wiki, and the real web UI

## Goal

Make Lattice useful to a team. Finish the schema language, build the pack framework, ship the Issues (Jira-class) and Wiki (Confluence-class) packs, and replace the dashboard prototype with a real UI on the API. From this phase on the project tracks its own work in Lattice.

## Scope

**In:** LSL v1 (facets, links, workflows, annotations, migrations), catalog versioning and SDK codegen, pack manifest and lifecycle, WASM plugin host, automations, saved views, Issues pack, Wiki pack, query language for work items, `lattice-web` on the real API, UI plugin API and the DB Schema plugin on real introspection, first importers.

**Out:** search beyond basic filters (Phase 7), agent intelligence (Phase 8), role packs beyond Issues and Wiki (Phase 9).

## Requirements

| ID | Requirement | Verified |
|---|---|---|
| P6-01 | LSL v1 grammar frozen by RFC: facets, composed types, links with cardinality and delete rules, workflows (states, transitions, guards, actions), annotations (`@merge`, `@phase`, `@label`, `@pii`, `@index`, `@extract`) | I, T |
| P6-02 | Compatibility checker and lens migrations used for every pack upgrade; breaking changes are rejected unless a migration is declared | P, S |
| P6-03 | Schema registry versioned in the catalog; SDKs regenerate from it; clients negotiate by schema hash | T |
| P6-04 | Pack manifest (name, version, requires, capabilities, LSL sources, workflows, views, automations); lifecycle Proposed, Checked, Installed, Active, Upgrading, Deprecated, Removed | T, S |
| P6-05 | WASM plugin host (wasmtime component model) with fuel and memory limits and capability-based host functions; a plugin cannot touch anything it was not granted | T, F |
| P6-06 | Automations: trigger, condition, action; actions run as `AutomationAction` plugins with the pack's grants; every automation write carries its author | T |
| P6-07 | Saved views and query language for work items (filter, sort, group) specified, parsed and compiled to SQL and live queries | T, P |
| P6-08 | Issues pack: epics, stories, tasks, bugs, workflows with guards, boards, backlog, sprints, estimates, assignees (people and agents), comments, links, attachments | T, D |
| P6-09 | Wiki pack: spaces, page tree with moves, templates, versions and diffs, inline and page comments, CRDT editing, attachments | T, D |
| P6-10 | `lattice-web` on the real API: board, list, ticket view, wiki, agents, drift placeholder, settings; keyboard accessible; WCAG 2.2 AA checks in CI | T, E |
| P6-11 | Web framework decision ADR with criteria (accessibility, bundle size, plugin API stability, contributor familiarity) | I |
| P6-12 | UI plugin API: register view, widget and command; versioned; the DB Schema plugin reimplemented on live introspection of connected databases | T, D |
| P6-13 | Importers as sidecars: Jira export, Confluence export, GitHub issues; import reports mapping fidelity and losses; imported history keeps original dates as valid time | T, E |
| P6-14 | Dogfood switch: from the gate date every new work item, document and decision of the Lattice project is created in Lattice; `backlog/` is imported and frozen | I, D |

## Tests, evals and benchmarks added

- UI end-to-end tests (Playwright) over the north-star flows, accessibility scans, visual regression.
- Workflow guard property tests; pack upgrade simulations with data present.
- Import round-trip evals: field mapping fidelity on real exports (consented) and synthetic ones, loss report must be explicit.
- Benchmarks: board render with 10,000 items, page load with large documents, collaborative edit latency.
- Fuzzing of the WASM host boundary and the query language.
- **Usability evals**: five scripted tasks (create a story, move through the workflow, write a page, assign to an agent, find something) with design partners; task success and time recorded on the scorecard.

## Exit criteria

| # | Criterion |
|---|---|
| E6-1 | Issues and Wiki packs install, upgrade and remove cleanly under simulation with data present |
| E6-2 | UI passes accessibility and e2e suites; the five usability tasks succeed for at least four of five design partners |
| E6-3 | Import evals meet fidelity thresholds set at the gate; losses reported |
| E6-4 | WASM host passes fuzzing and capability tests |
| E6-5 | **NS8 begins**: 100 percent of new project work lives in Lattice; documented friction list becomes backlog |
| E6-6 | Web framework ADR accepted; UI plugin API frozen for the 1.x line candidate |

## Depends on, risks

Depends on Phases 3, 4 and 5. Risks: UX fit (design partners and prototype-led research); plugin host security (capability model, fuzzing); LSL scope creep (RFC gate); dogfooding on an immature product (keep a documented fallback for a day-long outage).
