# ADR 0002: Native Rust engine with PostgreSQL wire compatibility

Status: accepted

## Context

A Postgres fork keeps C, a process-per-connection model, and a heap-page storage layer that conflicts with a unified WAL across log, row, graph, vector, text and blob engines.

## Decision

Build a native engine and speak the PostgreSQL wire protocol and a documented SQL subset so existing drivers and tools work. Revisit if the SQL pivot trigger in docs/spec/direction.md fires.

## Consequences

SQL coverage is a documented subset, not full Postgres. We gain one WAL and one transaction manager across all engines.
