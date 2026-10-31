#![no_main]
use lattice_api::auth::{Accounts, DecoySecret};
use lattice_seams::Rng;
use libfuzzer_sys::fuzz_target;

/// Fixed unpredictable-looking secret for the synthetic unknown-account verifiers. Any value of the
/// right length exercises the same code; a production host supplies a provisioned secret.
const DECOY: [u8; 32] = [0x5a; 32];
/// A name no generated file is expected to hold; its presence is checked before asserting on it.
const ABSENT: &str = "no-such-account";

/// Deterministic nonce source: the target only needs a value, never an unpredictable one.
struct Counter(u64);

impl Rng for Counter {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(1);
        self.0
    }
}

fuzz_target!(|data: &[u8]| {
    // Parse or refuse, never panic. A file that parses holds only provisioned accounts: the name
    // on each accepted line selects its own principal, and a name the file does not hold selects
    // none.
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let decoy = DecoySecret::from_bytes(&DECOY).expect("32 bytes");
    let Ok(accounts) = Accounts::parse(text, decoy) else {
        return;
    };
    let mut rng = Counter(0);
    for line in text.lines().filter(|line| !line.is_empty()) {
        let (name, _) = line.split_once('\t').expect("an accepted line has a tab");
        let (_, principal) = accounts
            .exchange(name, &mut rng)
            .expect("an accepted account name is valid");
        assert_eq!(principal.as_ref().map(|p| p.name()), Some(name));
    }
    let present = text.lines().any(|line| {
        line.split_once('\t')
            .is_some_and(|(name, _)| name == ABSENT)
    });
    if !present {
        let (_, principal) = accounts
            .exchange(ABSENT, &mut rng)
            .expect("the absent name is valid");
        assert!(
            principal.is_none(),
            "an absent account selected a principal"
        );
    }
});
