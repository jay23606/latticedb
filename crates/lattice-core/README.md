# lattice-core

Layer L3.

**Responsibility.** Kernel: typestate transactions, WAL format, policy traits, engine traits.

**Must not.** Open sockets, spawn background loops, know any pack, load models.

Allowed workspace dependencies: lattice-abi, lattice-seams, lattice-lsl.
