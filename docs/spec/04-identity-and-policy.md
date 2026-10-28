# Phase 4: identity, authorization and audit

## Goal

Make Lattice safe to share. Build the OAuth 2.1 and OIDC provider inside the database, relationship-based and attribute-based authorization compiled into scans, scoped agent grants, approval gates and a tamper-evident audit chain.

## Scope

**In:** OIDC provider, token flows, signing keys, MFA, SCIM, optional upstream federation, relationship tuples (ReBAC) and attribute rules (ABAC), policy compiler to row filters, proof-token plumbing, agent grants including read-only mode, approvals, audit chain, rate limiting and lockout.

**Out:** UI beyond login and consent pages, external security review (Phase 10).

## Requirements

| ID | Requirement | Verified |
|---|---|---|
| P4-01 | Endpoints: discovery, JWKS, authorize, token, userinfo, introspect, revoke, device authorization, dynamic client registration | C |
| P4-02 | Flows: authorization code with PKCE, client credentials, refresh with rotation and reuse detection, device code, token exchange (RFC 8693) with `act` delegation chain | C, T |
| P4-03 | OpenID conformance suite passes for the profiles we claim; unclaimed profiles are documented | C |
| P4-04 | Signing keys stored in the engine, rotated on schedule, old keys retained for verification; token verification needs no network | T |
| P4-05 | Passwords hashed with argon2id; MFA with WebAuthn and TOTP; lockout and rate limiting with tests | T |
| P4-06 | SCIM 2.0 provisioning for users and groups | C |
| P4-07 | Optional upstream federation (OIDC, SAML, LDAP) is a separately enabled component outside the inbound-only core guarantee, documented as an exception | I, T |
| P4-08 | Relationship tuples and attribute rules in the engine; check, expand and list APIs; schema for relations is LSL-declared | T, P |
| P4-09 | Policy compiler turns rules into scan filters used by every engine access; **I7 no bypass**: SQL, change feed, sync, MCP, and later S3, search and history all pass through it | S, P |
| P4-10 | `Authorized<Action>` proof tokens are the only way to reach engine write APIs; an attempt to write without one does not compile | T |
| P4-11 | Differential test: the optimized policy evaluator equals a simple, separate reference evaluator on generated tuples, rules and queries | P |
| P4-12 | Agent grants: scope, kinds, expiry, requires-approval flag, **read-only mode**; a read-only grant cannot produce a write by any path | S, E |
| P4-13 | Approvals: request entity, approver relation, expiry, live notification, decision recorded; approval cannot be self-granted by the requester | T |
| P4-14 | **I13 audit chain** records every authentication, grant, approval, denial and write with actor and on-behalf-of; verification job detects tampering | P, T |
| P4-15 | Threat model document and abuse-case tests: token theft, confused deputy through token exchange, injected instructions causing escalation, replay | I, T |
| P4-16 | Secrets and keys never appear in logs, errors or traces (tested with the Phase 1 secret-logging test) | T |

## Tests, evals and benchmarks added

- Security evals: a read-only agent attempts every write path (zero writes allowed); an agent receives a document containing injected instructions and attempts escalation (all blocked and audited).
- Differential and property tests of policy; fuzzing of JWT, OIDC parameters, SCIM payloads.
- Simulation with forged, expired, replayed and rotated tokens; clock skew.
- Benchmarks: policy check cost, filtered scan overhead against unfiltered, token verification throughput.
- North star **NS5** passes for the policy layer.

## Exit criteria

| # | Criterion |
|---|---|
| E4-1 | OpenID conformance suite passes for the claimed profiles |
| E4-2 | I7 differential test and simulation show zero bypasses across the budget; I13 verified |
| E4-3 | Security evals: zero writes from read-only agents, zero escalations, 100 percent of attempts audited |
| E4-4 | Policy overhead measured and within the budget set at the gate |
| E4-5 | Threat model reviewed by two maintainers; external review scheduled for Phase 10 |
| E4-6 | NS5 passes at the policy layer |

## Depends on, risks

Depends on Phase 1 (can run parallel to Phase 2). Risks: security surface of a built-in IdP (conformance, fuzzing, external review); policy performance (compile to filters, measure early); scope creep into enterprise features (SAML and LDAP stay optional).
