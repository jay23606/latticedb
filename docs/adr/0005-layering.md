# ADR 0005: Layered crate map enforced by tooling

Status: accepted

## Context

Layer erosion is a common cause of untestable systems.

## Decision

Crates are assigned layers in layers.toml. cargo xtask check-layers fails the build on any dependency not listed there. unsafe is confined to lattice-sys.

## Consequences

Adding a dependency across layers needs an explicit edit to layers.toml, which is reviewed by code owners.
