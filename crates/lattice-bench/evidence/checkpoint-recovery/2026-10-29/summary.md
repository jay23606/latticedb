# Checkpoint recovery measurement

Commit: `1d7035092b192a6f9104c7aa1930a65d7390a5e0`

fresh processes; shared warm/uncontrolled OS page cache; no cache drop; alternating mode order

Enforced cgroup ceiling: 192 MiB, swap disabled. Over-limit calibration produced an OOM kill.

RSS is whole fresh recover-process peak; cgroup peak also counts charged cache. Retained histories/indexes/dedupe grow.

| Dataset | Mode | Prefix bytes / files | Suffix bytes | State bytes | Median open ms | Peak RSS KiB (max) |
|---|---|---|---|---|---|---|
| baseline_minimal | full | 49 / 1 | 134 | 64 | 0.461 | 2612 |
| baseline_minimal | checkpoint | 49 / 1 | 134 | 64 | 0.602 | 2684 |
| retained_256 | full | 1116061 / 1 | 36198 | 1092376 | 6.755 | 5068 |
| retained_256 | checkpoint | 1116061 / 1 | 36198 | 1092376 | 3.514 | 4196 |
| retained_1024 | full | 4465157 / 3 | 36198 | 4370344 | 24.074 | 10336 |
| retained_1024 | checkpoint | 4465157 / 3 | 36198 | 4370344 | 10.900 | 8820 |
| retained_4096 | full | 17862537 / 9 | 36198 | 17483248 | 88.114 | 27632 |
| retained_4096 | checkpoint | 17862537 / 9 | 36198 | 17483248 | 43.388 | 26844 |
| payload_4k | full | 353829 / 1 | 36198 | 82088 | 2.191 | 2996 |
| payload_4k | checkpoint | 353829 / 1 | 36198 | 82088 | 0.958 | 2868 |
| payload_256k | full | 16869029 / 9 | 36198 | 82088 | 50.264 | 5428 |
| payload_256k | checkpoint | 16869029 / 9 | 36198 | 82088 | 0.937 | 2880 |
| payload_4m | full | 268528165 / 65 | 36198 | 82088 | 854.850 | 18816 |
| payload_4m | checkpoint | 268528165 / 65 | 36198 | 82088 | 1.163 | 2860 |
| segments_few | full | 8513013 / 2 | 36198 | 89256 | 33.522 | 10732 |
| segments_few | checkpoint | 8513013 / 2 | 36198 | 89256 | 1.020 | 2920 |
| segments_many | full | 8521269 / 518 | 36230 | 89256 | 32.186 | 2928 |
| segments_many | checkpoint | 8521269 / 518 | 36230 | 89256 | 2.824 | 2876 |
| suffix_zero | full | 4286021 / 3 | 134 | 82088 | 15.446 | 4760 |
| suffix_zero | checkpoint | 4286021 / 3 | 134 | 82088 | 0.824 | 2856 |
| suffix_4096 | full | 4286021 / 3 | 4616358 | 82088 | 33.582 | 9992 |
| suffix_4096 | checkpoint | 4286021 / 3 | 4616358 | 82088 | 21.879 | 9928 |

## Checkpoint I/O and retained state

| Dataset | Row / index versions | Committed IDs | Prefix reads | len calls | WAL entries visited | Largest read | Max cgroup peak bytes |
|---|---|---|---|---|---|---|---|
| baseline_minimal | 0 / 0 | 0 | 0 | 4 | 8 | 146 | 970752 |
| retained_256 | 1056 / 1755 | 1056 | 0 | 12 | 8 | 262144 | 2555904 |
| retained_1024 | 4128 / 7021 | 4128 | 0 | 38 | 12 | 262144 | 7266304 |
| retained_4096 | 16416 / 28086 | 16416 | 0 | 144 | 24 | 262144 | 25882624 |
| payload_4k | 288 / 365 | 352 | 0 | 4 | 8 | 82088 | 1241088 |
| payload_256k | 288 / 365 | 352 | 0 | 12 | 24 | 82088 | 1245184 |
| payload_4m | 288 / 365 | 352 | 0 | 68 | 136 | 82088 | 1253376 |
| segments_few | 288 / 365 | 800 | 0 | 5 | 10 | 89256 | 1245184 |
| segments_many | 288 / 365 | 800 | 0 | 521 | 1046 | 89256 | 1241088 |
| suffix_zero | 256 / 365 | 320 | 0 | 6 | 12 | 82088 | 1249280 |
| suffix_4096 | 4352 / 365 | 4416 | 0 | 6 | 16 | 2097481 | 8323072 |
