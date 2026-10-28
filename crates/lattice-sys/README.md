# lattice-sys

Layer L0.

**Responsibility.** The only crate allowed to contain unsafe code (io_uring, mmap wrappers).

**Must not.** Contain business logic, or be used except through lattice-seams.

Allowed workspace dependencies: none.
