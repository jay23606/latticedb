# Phase 12: general availability and open-source launch

## Goal

Ship 1.0 with promises we can keep: stable wire protocol, on-disk format and schema language, documented operations, a security process, and a community that can contribute.

## Scope

**In:** compatibility and support policy, packaging, install and upgrade experience, documentation completeness, migration guides, benchmark publication with methodology, reproducible builds and SBOM, security policy and response drill, governance and community setup, release process and long-term support.

**Out:** features that did not pass their phase gate.

## Requirements

| ID | Requirement | Verified |
|---|---|---|
| P12-01 | Compatibility policy: semver for wire protocol, on-disk format, LSL grammar, SDKs and plugin ABI; deprecation window; supported upgrade paths; written and linked from every release note | I |
| P12-02 | Freeze corpus covers every format shipped in a pre-1.0 release that users might hold; the 1.0 reader reads all of them or a documented migration does | T |
| P12-03 | Packaging: static Linux binaries (x86_64, aarch64), container image, package manager recipes; each signed with SBOM and provenance attestation | T |
| P12-04 | Reproducible build check: two independent builds of the same commit produce identical artifacts or a documented, bounded difference | T |
| P12-05 | Install in under five minutes on a clean machine; first-run wizard creates tenant, admin and sample workspace; measured with unassisted users | D, E |
| P12-06 | Documentation complete: concepts, quick start, operations, security, SQL subset, S3 subset, SDKs, plugin contract, LSL reference, runbooks, FAQ; every code sample is tested | T, I |
| P12-07 | Migration guides from Jira, Confluence, Linear and GitHub; importer docs with known losses | I |
| P12-08 | Security: vulnerability disclosure process exercised in a drill; CVE handling; dependency advisory response time target | D |
| P12-09 | Benchmarks published with hardware, workload, scripts and raw data; no claim without reproducibility; comparison with PostgreSQL and MinIO limited to declared workloads | I, B |
| P12-10 | Governance: maintainers, release managers, decision process, contributor ladder, trademark and naming policy, code of conduct enforcement path | I |
| P12-11 | Release process: release candidates, soak on dogfood instance, canary on design partner instances, staged rollout, rollback plan | D |
| P12-12 | Long-term support policy and the 1.x maintenance branch workflow | I |
| P12-13 | Final north-star run: NS1 to NS8 all pass on release candidates | D |

## Tests, evals and benchmarks added

- Release candidate suite: full nightly and weekly suites on the candidate, upgrade matrix from every supported earlier version, install tests on supported platforms.
- Documentation tests (all samples run).
- Usability evals repeated with fresh external users for install and first task.
- Final benchmark publication run on the declared reference hardware.

## Exit criteria

| # | Criterion |
|---|---|
| E12-1 | NS1 to NS8 pass on the release candidate |
| E12-2 | All scorecard areas at or above thresholds for four consecutive weeks |
| E12-3 | No open high or critical security finding; disclosure drill completed |
| E12-4 | Upgrade matrix green; freeze corpus green |
| E12-5 | Unassisted install and first task succeed for at least four of five external users |
| E12-6 | Documentation tests green; migration guides reviewed by design partners |
| E12-7 | Two maintainers sign the 1.0 gate; compatibility policy published |

## Depends on, risks

Depends on Phases 10 and 11 (11 may end as a documented no-go and still allow GA). Risks: pressure to ship features that missed their gate (the gate process and waiver rules are the defence); promises that outlive their evidence (compatibility policy is limited to what the freeze corpus and upgrade matrix verify).
