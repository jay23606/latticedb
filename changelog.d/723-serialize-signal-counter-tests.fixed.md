The `lattice-sys` signal counter tests no longer fail intermittently when test threads run in parallel. Their process-wide counters are now read one test at a time. Runtime behavior is unchanged.
