# ADR 0004: Models are an optional plugin, off by default

Status: accepted

## Context

Agents can perform extraction and judging through a work queue. The GLiNER models are an accelerator, not a dependency.

## Decision

Intelligence is a slot. Agents fill slots by default. The lattice-intel plugin is behind feature intel, off by default, and CI must pass with it disabled.

## Consequences

The core stays small and weight-free. Some slots are slower and costlier with agents until the plugin exists.
