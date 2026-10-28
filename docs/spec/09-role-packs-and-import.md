# Phase 9: role packs and importers

## Goal

Make one tool serve the whole team. Add the packs each role needs and prove the claim with design partners running real sprints in Lattice.

## Scope

**In:** Specs and BA pack, Design pack, Planning and PM pack, Ops and Release pack, QA pack, Support and Customer pack, templates, automation library, reports, importers for GitHub, Linear and others, role-based onboarding.

**Out:** cluster work (Phase 10), anything requiring models beyond agents.

## Requirements

| ID | Requirement | Verified |
|---|---|---|
| P9-01 | **Specs and BA**: requirements, user stories, acceptance criteria, glossary, process models, decision records (ADR-style), stakeholder map; traceability matrix from requirement to story to test to deployment as one query | T, E |
| P9-02 | **Design**: assets stored through the blob engine with versions, design tokens as typed entities, component registry, region-pinned review comments, handoff specs, token-versus-implementation drift kind | T, E |
| P9-03 | **Planning and PM**: roadmap, OKRs, milestones, capacity and time tracking, risk and dependency register, auto-generated status reports with citations, DORA metrics from deployment entities | T, E |
| P9-04 | **Ops and Release**: service catalog, environments, deployments (state machine), feature flags, incidents, postmortems, SLOs, on-call schedule, change calendar | T, S |
| P9-05 | **QA**: test cases, plans, runs, results, flaky test tracker, coverage links to requirements | T |
| P9-06 | **Support and Customer**: feedback and tickets, insights linked to roadmap items | T |
| P9-07 | Each pack is installed, upgraded and removed through the pack lifecycle without touching the kernel; each declares its phases, labels and drift kinds | T, S |
| P9-08 | Role-based starting views and templates for each role; onboarding path of at most ten minutes to a first useful action, measured | D, E |
| P9-09 | Automation library: common rules (assign by component, link PR to ticket, move on merge) as shipped, auditable automations | T |
| P9-10 | Status reports and traceability answers are grounded: every figure and citation resolves to existing, permitted entities; reports are reproducible from a snapshot | E |
| P9-11 | Importers for GitHub, Linear and additional trackers; fidelity and loss reports; importers run outside the core | T, E |
| P9-12 | Cross-role permission model examples shipped (client-visible items, designer-only assets, finance-only fields) and tested against I7 | T, S |
| P9-13 | **NS6**: a PM, BA, designer, developer and two agents complete a sprint in Lattice with design partners | D |

## Tests, evals and benchmarks added

- Per-pack workflow property tests and upgrade simulations.
- **Role task evals** with design partners: scripted tasks per role with success, time and error counts on the scorecard.
- Report grounding evals (figures recomputed from the database and compared).
- Importer fidelity evals on real exports.
- Benchmarks with a realistic mixed workload: 50 users, 10 agents, 1 million entities.

## Exit criteria

| # | Criterion |
|---|---|
| E9-1 | NS6 passes with at least three design partners and one external team |
| E9-2 | Role task evals meet the thresholds set with partners; onboarding time within target |
| E9-3 | Report and traceability grounding at 100 percent for numbers, citation thresholds met |
| E9-4 | All packs pass install, upgrade and removal simulations |
| E9-5 | Mixed-workload benchmark recorded with no regression against Phase 7 baselines beyond budget |

## Depends on, risks

Depends on Phases 6 and 8. Risks: pack sprawl (each pack must justify itself with a partner task); role-specific permission mistakes (I7 tests on every pack); scope drift towards a clone of existing tools (the north star is traceability and freshness, not feature parity).
