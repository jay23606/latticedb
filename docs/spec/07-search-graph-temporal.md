# Phase 7: text, vector, graph and temporal search

## Goal

Make everything findable by text, meaning, relationships and time, including history questions answered exactly: "when did this feature start being built?" and "when did I create t.go?".

## Scope

**In:** text engine (BM25), vector engine (bring-your-own vectors, disk-resident ANN), graph engine (CSR plus temporal edge log), temporal engine (lifespans, path lineage, activity buckets), the query planner for hybrid and temporal intents, SQL functions and MCP tools, time-pruned segments.

**Out:** extraction and judging (Phase 8), embedders (Phase 11); vectors are supplied by clients or agents until then.

## Requirements

| ID | Requirement | Verified |
|---|---|---|
| P7-01 | Text engine with BM25, phrase and prefix queries, field filters, time-segment pruning; maintained in the same transaction as the change | P, S |
| P7-02 | Vector engine with filtered ANN, configurable dimensions and metric, disk-resident index, bring-your-own vectors with provenance recorded per vector | P, B |
| P7-03 | Graph engine: adjacency in CSR form plus a temporal edge log (`valid_from`, `valid_to`), incremental update and full rebuild that agree | P, S |
| P7-04 | Graph query surface per ADR (SQL graph patterns or Cypher subset), including `AS OF` | T, C |
| P7-05 | Temporal engine maintained in the commit transaction: `entity_lifespan`, `path_lineage`, `activity_bucket`, `time_segment` | S |
| P7-06 | **I14**: every lifespan, lineage and activity answer equals a scan of the log, across crash and fault injection | S, P |
| P7-07 | **I9** scrubbers for text, vector, graph and temporal structures | S, T |
| P7-08 | Planner for intents `find`, `created`, `first_seen`, `last_seen`, `timeline`, `lifespan`, `as_of`, `activity`; relevance first, then time-aware aggregation | T, E |
| P7-09 | Rank fusion (reciprocal rank fusion) across text, vector and graph proximity; vectors optional, fallback documented and tested | E |
| P7-10 | Phases (`@phase`) drive `timeline` milestones; the answer states which phase defined "started" | T |
| P7-11 | Path lineage across moves, delete and recreate, and multiple lineages; creator resolved from the caller's principal | S, T |
| P7-12 | Valid time versus transaction time handled for imported history; both reported in answers | T |
| P7-13 | **I7** for all of the above: results, counts, aggregates and "first" or "last" are computed after the policy filter | S, P |
| P7-14 | **I10** cascade: purging an entity removes its vectors, index entries, graph edges, lifespan, lineage and activity contributions | S |
| P7-15 | SQL functions `search`, `history_search`, `path_history`; MCP tools with the same semantics and a structured query schema | T |

## Tests, evals and benchmarks added

- **Search evals**: query sets per role (PM, BA, designer, developer), metrics nDCG@10, MRR, recall@k; with and without vectors; thresholds set from Phase 1 harness.
- ANN recall against exact search at declared latency budgets; filter correctness properties.
- Temporal exactness: indexed answer equals log scan on generated histories with renames, deletes, imports and clock skew.
- Policy evals: users with different grants must get different "first" and "last" answers where hidden items exist.
- Benchmarks: query latency by corpus size, index maintenance overhead on write path, cold start of indexes.
- North star **NS2** passes.

## Exit criteria

| # | Criterion |
|---|---|
| E7-1 | I14, I9 and I10 verified in simulation across 1,000,000 seeds per night for seven nights |
| E7-2 | NS2 passes with exact dates, actors and evidence; permission variants correct |
| E7-3 | Search evals meet thresholds, with and without vectors |
| E7-4 | ANN recall and latency within the declared budget on the benchmark corpora |
| E7-5 | Write-path overhead of all indexes measured and within the budget set at the gate |
| E7-6 | Graph query language ADR accepted |

## Depends on, risks

Depends on Phases 2 to 6 (policy path from Phase 4, packs and phases from Phase 6). Risks: index write amplification (budget and benchmarks); eval datasets that flatter results (consented real samples, per-role query sets); temporal semantic ambiguity (phases explicit, answers show their definition).
