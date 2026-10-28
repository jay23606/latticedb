# latticed

Layer L7.

**Responsibility.** Composition root: wires implementations to traits and runs the server.

**Must not.** Contain logic that belongs in a library crate.

Allowed workspace dependencies: lattice-abi, lattice-seams, lattice-sys, lattice-core, lattice-engines, lattice-identity, lattice-service, lattice-packs, lattice-api.
