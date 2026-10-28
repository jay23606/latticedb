# ADR 0003: The core makes no outbound connections

Status: accepted

## Context

The single-database goal and security posture require that the core is auditable and never dials out. Connectors, importers, email delivery and identity federation need egress.

## Decision

The core, API, service and engine crates never open outbound connections. Egress lives in optional sidecars and one explicitly separate federation component. The core never calls an LLM.

## Consequences

Notification delivery uses an outbox drained by a sidecar. Upstream identity federation is a documented exception, enabled separately.
