# evals

Eval suites live in `evals/<capability>/<suite>/` with `dataset/`, `config.toml` and `thresholds.toml`.
Datasets carry a manifest with a content hash. Thresholds are reviewed by eval owners.

`thresholds.toml` format:

```toml
[[metric]]
name = "f1"
lower_bound = 0.80
confidence = 0.95
tolerance = 0.01
```
