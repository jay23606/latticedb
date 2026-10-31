# Lattice Go client

Status: experimental. Third client slice of WP51 ([#253](https://github.com/moncifem/latticedb/issues/253)),
after the [Python client](../python) and the [TypeScript client](../typescript). The formats it implements are
unaccepted candidates ([sync protocol v1](../../formats/sync-protocol-v1.md),
[change feed v1](../../formats/change-feed-v1.md)); expect breaking changes. The module is not published.

This slice is the wire codecs only: the sync frames (`Hello`, `Welcome`, `Subscribe`, `Events`, `Ack`, `Error`,
`Ping`, `Pong`), `Negotiate`, the 12-byte cursor and the change event. The client state machine
(`clients/python/lattice_client/client.py`) is not ported yet.

It needs Go 1.22 or later, uses only the standard library and has no network code. The code is written by hand
and kept identical to the Rust implementation by the shared vectors in
[`conformance/vectors.json`](../../conformance/vectors.json).

## Layout

| File | Contents |
|---|---|
| `lattice/sync.go` | Encode and decode of every sync frame, `Negotiate`, the field limits. A port of `lattice_api::sync`. |
| `lattice/feed.go` | The cursor, its order, and encode and decode of a change event. A port of `lattice_core::feed`. |
| `lattice/bytes.go` | The bounded reader and the control character check the codecs share (private). |

## Decoding is strict

The rules are the Rust ones, in the same order, so the same bytes fail with the same error:

- the frame type is judged from the first byte and the length from the first five, before the payload is read;
  a length above the limit (1 MiB, or the one `Welcome` announced) is refused;
- every field is checked against its bounds; text must be valid UTF-8 without control characters; reserved
  values (subscription 0, version 0, error codes outside 1 to 7) are refused;
- bytes left after the last field are an error, and so is any non-canonical form;
- fewer bytes than a whole frame return no message and no error ("wait for more").

Decoded byte fields are copies and never alias the input. Errors are `*SyncError`, `*FeedError` and
`*FeedDecodeError`. Each has a `Kind` that matches the Rust variant, and a `Value` or `Field` where Rust
carries one.

## Test

```sh
cd clients/go
gofmt -l .          # prints nothing when formatted
go vet ./...
go test -count=1 ./...
```

The tests read every vector in `conformance/vectors.json`: each valid frame and event must decode to the
expected value and encode back to the same bytes, every proper prefix of a valid frame must be incomplete,
and every invalid input must fail with the kind and detail Rust reports. The job is `.github/workflows/clients-go.yml`.
