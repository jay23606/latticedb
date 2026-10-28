# Format: hybrid logical clock

A timestamp is `(wall_ms u64, counter u32, node u32)` ordered lexicographically. Encoded as 16 bytes
little endian in that order.

* **Local event** at wall-clock `now`: `wall = max(now, last.wall)`; the counter is `last.counter + 1` if
  `wall == last.wall`, else 0. On counter overflow the wall part increases by 1 and the counter is 0.
* **Receive** remote `r` at `now`: refuse if `r.wall - now` exceeds the drift bound; `wall = max(now,
  last.wall, r.wall)`; the counter continues from the largest counter among values sharing that wall.
* **Properties** (tested): strictly increasing per node whatever the wall clock does; a merged timestamp
  is after both inputs; physical drift is bounded.
