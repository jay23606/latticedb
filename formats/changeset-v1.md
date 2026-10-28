# Format: ChangeSet v1

Status: accepted (ADR 0007). Payload of a WAL record of type 1.

```
version u8 = 1
id 16 bytes (big endian ULID)
hlc 16 bytes: wall_ms u64 | counter u32 | node u32 (little endian)
author 16 bytes (big endian)
turn flag u8: 0 none, 1 present; if present: session u64 | turn u64
parent count u16, then that many 16 byte ids
payload length u32, then the payload bytes
```

Trailing bytes after a complete change set are an error. A `TurnBegin` record (type 2) has the payload
`session u64 | turn u64`.
