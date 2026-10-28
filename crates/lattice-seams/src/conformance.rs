//! A conformance suite every [`Io`] implementation, real or simulated, must pass.

use crate::{Io, IoError};

/// Runs the contract checks against `io`. Panics with a clear message on the first violation.
#[allow(clippy::panic, clippy::expect_used)]
pub fn run<I: Io>(io: &mut I) {
    io.create_dir_all("conf").expect("create dir");
    assert!(!io.exists("conf/a"), "fresh file must not exist");
    assert_eq!(io.len("conf/a"), Err(IoError::NotFound("conf/a".into())));

    io.append("conf/a", b"hello").expect("append");
    io.append("conf/a", b" world").expect("append");
    assert_eq!(io.read("conf/a").expect("read"), b"hello world");
    assert_eq!(io.len("conf/a").expect("len"), 11);

    io.sync("conf/a").expect("sync");
    io.sync_dir("conf").expect("sync dir");

    io.truncate("conf/a", 5).expect("truncate");
    assert_eq!(io.read("conf/a").expect("read"), b"hello");

    io.append("conf/b", b"x").expect("append b");
    assert_eq!(
        io.list("conf").expect("list"),
        vec!["a".to_string(), "b".to_string()]
    );

    io.remove("conf/b").expect("remove");
    assert!(!io.exists("conf/b"));
    assert_eq!(io.remove("conf/b"), Err(IoError::NotFound("conf/b".into())));
    assert_eq!(io.list("conf").expect("list"), vec!["a".to_string()]);

    // Locks exclude a second taker until released.
    io.lock("conf/LOCK").expect("first lock");
    assert!(
        io.lock("conf/LOCK").is_err(),
        "a held lock cannot be taken again"
    );
    io.unlock("conf/LOCK");
    io.lock("conf/LOCK").expect("lock after unlock");
    io.unlock("conf/LOCK");
}
