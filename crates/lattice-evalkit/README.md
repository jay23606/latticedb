# lattice-evalkit

Layer dev. Eval runner, metrics, datasets, reports.

Planned modules (Phase 1): `dataset` (manifest and hash-checked loader), `provider` and `suite` traits,
`metrics` (F1, B-cubed, ECE, nDCG, bootstrap intervals), `gate` (thresholds), `report` (JSON and HTML).
Owner: eval owners. Must not: be a normal dependency of any non-dev crate.
