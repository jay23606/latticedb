# benches

Workloads, baselines and hardware profiles. A hardware profile is `profiles/<name>.toml`:

```toml
name = "reference-linux-x86"
cpu = "describe the CPU"
memory_gb = 64
storage = "NVMe, describe"
```

Baselines are stored per profile under `baselines/<profile>/`.
