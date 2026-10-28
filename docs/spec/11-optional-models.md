# Phase 11: optional GLiNER plugin

## Goal

Offer small on-box models as an **optional accelerator** for the slots agents already fill. The plugin is **off by default**, the core never depends on it, and it must beat the agent baseline on cost or latency without losing quality, measured by the same evals.

## Scope

**In:** the `lattice-intel` crate behind feature `intel`, an ONNX runtime wrapper, GLiNER2.5 extractor and GLiNER2.5-Decide judge as providers for the same slots, an optional embedder, model registry in the blob store, offline installation, resource limits.

**Out:** hosted model calls of any kind; training pipelines.

## Requirements

| ID | Requirement | Verified |
|---|---|---|
| P11-01 | Feature `intel` is off by default; the full test suite, all north stars and all evals pass with it disabled (CI job named "intel-off" is required) | T |
| P11-02 | The core, API, service and engines compile without `lattice-intel` and contain no reference to ONNX or model weights | T |
| P11-03 | ONNX export and Rust inference parity for GLiNER2.5 and GLiNER2.5-Decide: outputs match the reference implementation within a documented tolerance on a fixed validation set | T, E |
| P11-04 | Licence verification for every bundled checkpoint, recorded in the model registry manifest; unlicensed or unverifiable weights cannot be installed | I, T |
| P11-05 | `GlinerExtractor` implements `Extractor` and `DecideJudge` implements `Judge`, returning the same result shapes and passing the same schema validation as agent providers | T |
| P11-06 | Bindings choose a provider per slot (`agent`, `rule`, `gliner2.5`, `decide`); the Phase 8 shadow, assist and auto rules apply unchanged | T, E |
| P11-07 | Model registry in the blob store with content hashes; offline install; verified at load | T |
| P11-08 | Resource limits: inference thread pool with backpressure, memory cap, CPU affinity option; inference never blocks the commit path | S, B |
| P11-09 | Relation extraction chunking and context policy tuned and evaluated, because recall degrades on long inputs | E |
| P11-10 | Optional embedder fills the vector slot; vectors carry model provenance and are rebuilt when the embedder changes | T, E |
| P11-11 | A/B evals: paired comparison of plugin versus agent on the Phase 8 suites with bootstrap intervals; the report states when each provider wins | E |
| P11-12 | Cost and latency report per provider on the reference workload | B |

## Tests, evals and benchmarks added

- Parity tests against the reference implementation; golden outputs.
- The full Phase 8 eval suites rerun with each provider; calibration recomputed per provider, never reused across providers.
- Backpressure and isolation simulations: inference stalls must not delay commits.
- Benchmarks: items per second per core, memory, cold start.
- Fuzzing of the ONNX input preparation and chunker.

## Exit criteria

| # | Criterion |
|---|---|
| E11-1 | Intel-off CI job green on every change since the phase began |
| E11-2 | Parity within tolerance; licences verified |
| E11-3 | A/B report published; the plugin is recommended only for slots where it meets quality thresholds and improves cost or latency |
| E11-4 | Inference isolation verified: p99 commit latency unchanged under full inference load |

## Depends on, risks

Depends on Phase 8. Risks: ONNX export or operator support gaps (spike first, fallback to a different runtime or stay agent-only; this phase may end with a documented no-go); licence constraints; model updates changing graph content (ExtractorRun and Judgment records keep results attributable and re-runnable).
