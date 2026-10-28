# lattice-api

Layer L6.

**Responsibility.** Protocol front doors: PG wire, S3, sync, OIDC endpoints, MCP, gRPC.

**Must not.** Contain business rules.

Allowed workspace dependencies: lattice-abi, lattice-seams, lattice-core, lattice-engines, lattice-identity, lattice-service, lattice-packs, lattice-lsl.
