//! The harness over real sockets: the scenario every later end-to-end package builds on, and the
//! seeded kill schedule that the reconnect packages will drive.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use lattice_e2e::{Fixture, Ws, kill_points, row_value};

#[test]
fn harness_starts_latticed_and_two_principals_see_a_row_written_by_one() {
    let fixture = Fixture::start("harness").unwrap();
    fixture.seed(1..=2).unwrap();

    let mut alice = Ws::open(fixture.ws, &fixture.tokens.alice).unwrap();
    alice.hello("alice-client").unwrap();
    alice.subscribe(1, "table/notes").unwrap();
    let mut bob = Ws::open(fixture.ws, &fixture.tokens.bob).unwrap();
    bob.hello("bob-client").unwrap();
    bob.subscribe(1, "table/notes").unwrap();

    // Both subscriptions start from the beginning of the table and see the seeded rows.
    for ws in [&mut alice, &mut bob] {
        let rows: Vec<String> = ws
            .events(1, 2)
            .unwrap()
            .iter()
            .filter_map(row_value)
            .collect();
        assert_eq!(rows, ["v0001", "v0002"]);
    }

    // A row committed later, written through the SQL listener, reaches both live subscriptions.
    fixture
        .writer()
        .unwrap()
        .run("INSERT INTO notes VALUES (3, 'v0003')")
        .unwrap();
    for ws in [&mut alice, &mut bob] {
        let rows: Vec<String> = ws
            .events(1, 1)
            .unwrap()
            .iter()
            .filter_map(row_value)
            .collect();
        assert_eq!(rows, ["v0003"]);
    }
}

#[test]
fn kill_schedule_is_reproducible_from_its_seed() {
    let points = kill_points(7, 16);
    assert_eq!(points.len(), 16);
    assert_eq!(points, kill_points(7, 16));
    assert_ne!(points, kill_points(8, 16));
    assert!(points.windows(2).all(|w| w[0] < w[1]));
    // SplitMix64 from state 0 yields 0xe220a8397b1dcdaf, the published reference output. The first
    // point is 1 + (0xaf % 64) = 48.
    assert_eq!(kill_points(0, 1), [48u64]);
}
