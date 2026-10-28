# Phase 8: agent-driven intelligence and drift

## Goal

Build the knowledge graph and drift detection so that **agents do the extracting and judging by default**, through a work queue, with every result schema-validated, audited and measurable. The core still never calls a model.

## Scope

**In:** work queue (leases, results, schemas), `AgentExtractor`, `AgentJudge`, `RuleExtractor`, `RuleJudge`, entity resolver, mentions, assertions, anchors, bindings with shadow, assist and auto modes, judgments store and calibration, staged drift engine, reconcilers, connectors host with git and CI/CD connectors, notification outbox, the read-only "Ask an agent" flow, drift UI.

**Out:** the optional GLiNER plugin (Phase 11).

## Requirements

| ID | Requirement | Verified |
|---|---|---|
| P8-01 | Work queue: enqueue, claim with lease, submit, release, expire, retry limits, priorities, deadlines; state machine per the architecture; exactly-once application of a result | S, P |
| P8-02 | Result schemas generated from LSL per work kind (`extract`, `judge`, `triage`, `reconcile`); invalid submissions are rejected with violations and the item returns to the queue (**I8** for agent output) | T, F |
| P8-03 | Agent output is data, never instructions: submissions pass the same policy and schema gates as any write, under the submitting agent's grants | S, E |
| P8-04 | `RuleExtractor`: ids, links, versions, known aliases; deterministic and always on | T |
| P8-05 | `AgentExtractor` creates extract work items from changed text chunks (debounced, only changed chunks) with labels from `@label` annotations and nearby known entities as hints | T, E |
| P8-06 | Resolver: exact alias, then BM25 candidates, optional vectors, graph-context rerank, then a judge slot; outcomes accept, review or reject by calibrated bands | T, E |
| P8-07 | Assertions with stance (`claims`, `observes`), origin, confidence, valid time, provenance; authority ladder Candidate, Confirmed, Promoted, Superseded; human-confirmed assertions are never overwritten by re-extraction | P, T |
| P8-08 | Bindings per slot with provider, mode (shadow, assist, auto), thresholds and label set; promotion rules require measured precision and override rate; automatic demotion on distribution shift | T, E |
| P8-09 | Every judgment is recorded with input hash, provider, provider version, label scores, band, outcome and override; overrides feed calibration | T |
| P8-10 | Drift engine stages: impact set via anchors and graph, deterministic typed comparison, then claim extraction and judgment for prose changes | T, E |
| P8-11 | Drift kinds at launch limited to those with deterministic comparators plus doc-versus-deployment; each kind has an owner and a documented noise budget | I, E |
| P8-12 | Reconcilers propose fixes on a branch; acceptance merges and re-verifies the claim; dismissals are recorded as feedback | T |
| P8-13 | Connectors host (inbound only): webhook receiver sidecar, git, CI/CD and deployment connectors write Deployment and Commit entities as `observes` assertions | T |
| P8-14 | Notification outbox drained by an optional delivery sidecar; the core makes no outbound call | T, I |
| P8-15 | "Ask an agent": read-only questions assigned to an agent under a temporary read-only grant; answers carry citations and confidence; conversion to ticket or doc | T, E |
| P8-16 | Queue SLOs and dashboards: queue age p95, completion rate, schema rejection rate; degradation behaviour when no agent is online (deterministic stages continue) | S, T |
| P8-17 | **I7** for the intelligence layer: judges and extractors only ever see what their grants allow | S, E |

## Tests, evals and benchmarks added

- **Extraction evals**: precision, recall, F1 per label on hand-labelled sets; schema-valid rate (hard gate); latency and cost per item.
- **Resolution evals**: pairwise precision and recall, B-cubed, **false-merge rate weighted above false-split**.
- **Judgment evals**: accuracy per label, ECE and reliability, precision at the auto threshold, review rate, risk-coverage curve.
- **Drift evals**: mutation-injected drifts (changed number, renamed service, contradicted sentence, moved file); recall on injected drift, false positives per 1,000 changes, time to detect, severity accuracy, proposal acceptance rate.
- **Grounding evals** for read-only answers: every citation exists, is permitted, and contains the supporting span (automatic).
- **Security evals**: injected instructions in documents and tickets, read-only agents attempting writes.
- Shadow-mode infrastructure comparing provider outputs with human outcomes using paired tests.
- Simulation of queue behaviour: lease expiry, duplicate submission, agent crash mid-item, agent takeover.
- Benchmarks: queue throughput, resolver latency, drift pipeline throughput per 1,000 changes.
- North stars **NS3**, **NS4**, **NS5** (agent part) pass.

## Exit criteria

| # | Criterion |
|---|---|
| E8-1 | Queue invariants (exactly-once application, lease safety) hold in simulation across 1,000,000 seeds per night for seven nights |
| E8-2 | Schema-valid submission rate gate enforced; invalid never applied |
| E8-3 | Eval thresholds met on frozen sets with confidence intervals, thresholds frozen by ADR at the gate |
| E8-4 | Drift: recall and false-positive rate within the noise budget for each launched kind; unlaunched kinds are documented |
| E8-5 | Grounding: 100 percent of citations in the eval set exist and are permitted; at least the threshold contain the supporting span |
| E8-6 | Security evals pass: zero writes from read-only agents, zero escalations |
| E8-7 | NS3, NS4 and NS5 pass end to end |
| E8-8 | Shadow-to-assist promotion rules exercised on at least one real binding with recorded evidence |

## Depends on, risks

Depends on Phases 6 and 7. Risks: agent cost and latency (debounce, only changed chunks, queue SLOs, plugin fallback in Phase 11); poorly calibrated agent confidence (calibrate from overrides, start in shadow); drift noise (launch deterministic kinds first, noise budgets are product requirements); prompt injection (agent output is data, policy gates are the defence, security evals are the proof).
