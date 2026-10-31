#![no_main]
use lattice_api::auth::{Accounts, DecoySecret};
use lattice_api::pgwire::{Backend, Column, Startup, decode_frontend_sasl};
use lattice_api::principal::QueryContext;
use lattice_api::session::{Outcome, QueryError, QueryHandler, QueryOutput, Session};
use lattice_seams::Rng;
use libfuzzer_sys::fuzz_target;

/// The RFC 7677 verifier for the password `pencil` (salt `W22ZaJ0SNY7soEsUEjb6gQ==`, 4096
/// iterations), derived outside this target from the published vector and written as a credential
/// file line would be.
const VERIFIER: &str = "SCRAM-SHA-256$4096:W22ZaJ0SNY7soEsUEjb6gQ==$WG5d8oPm3OtcPnkdi4Uo7BkeZkBFzpcXkuLmtbsT4qY=:wfPLwcE6nTWhTAmQ7tl2KeoiWGPlZqQxSrmfPwDl2dU=";
const DECOY: [u8; 32] = [0x5a; 32];

/// Counts queries that reach the handler. No query may run before authentication.
#[derive(Default)]
struct Refuse {
    calls: usize,
}

impl QueryHandler for Refuse {
    fn execute(
        &mut self,
        _: &QueryContext<'_>,
        _: &str,
        _: &[Option<Vec<u8>>],
    ) -> Result<QueryOutput, QueryError> {
        self.calls += 1;
        Err(QueryError::new("42601", "no query before authentication"))
    }

    fn describe(&mut self, _: &QueryContext<'_>, _: &str) -> Result<Vec<Column>, QueryError> {
        self.calls += 1;
        Ok(Vec::new())
    }
}

/// Deterministic nonce source. The fuzzer cannot read the server nonce, so it cannot build a proof.
struct Counter(u64);

impl Rng for Counter {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(1);
        self.0
    }
}

/// Checks one reply. Each message encodes; nothing authenticates; a failure closes the connection.
fn check(out: &Outcome, session: &Session<Refuse>) {
    for message in &out.messages {
        assert!(message.encode(&mut Vec::new()).is_ok());
        assert!(
            !matches!(message, Backend::AuthenticationOk),
            "AuthenticationOk without a verified proof"
        );
    }
    if out
        .messages
        .iter()
        .any(|m| matches!(m, Backend::ErrorResponse { .. }))
    {
        assert!(
            out.close,
            "an error before authentication must end the session"
        );
    }
    if out.close {
        assert!(session.is_closed());
    }
    assert!(session.principal().is_none());
    assert_eq!(session.handler().calls, 0);
}

fuzz_target!(|data: &[u8]| {
    // Byte 0 picks the provisioned account (bit 0: `user` or `other`) and the startup user (bit 1).
    // The rest is wire bytes for the SASL phase. Account selection goes through `Accounts::exchange`
    // as a host does it, so the principal comes from the store, never from the client.
    let Some((&control, frames)) = data.split_first() else {
        return;
    };
    let account = if control & 1 == 1 { "user" } else { "other" };
    let startup_user = if control & 2 == 2 { "user" } else { "other" };
    let file = format!("{account}\t{VERIFIER}\n");
    let decoy = DecoySecret::from_bytes(&DECOY).expect("32 bytes");
    let accounts = Accounts::parse(&file, decoy).expect("the published verifier line parses");
    let (exchange, principal) = accounts
        .exchange(startup_user, &mut Counter(0))
        .expect("the startup user is a valid name");

    let mut session = Session::with_scram(Refuse::default(), 17, 23, exchange, principal);
    let started = session.on_startup(Startup::Params(vec![("user".into(), startup_user.into())]));
    check(&started, &session);

    // Each frame is read with the decoder for the phase the session is in. A malformed frame gets
    // the same answer a protocol error gets on the wire.
    let mut at = 0;
    while !session.is_closed() {
        let Some(rest) = frames.get(at..) else {
            break;
        };
        let Some(phase) = session.sasl_phase() else {
            break;
        };
        let out = match decode_frontend_sasl(rest, phase) {
            Ok(Some((message, used))) => {
                at += used.max(1);
                session.on_message(message)
            }
            Ok(None) => break,
            Err(error) => session.on_protocol_error(&error),
        };
        check(&out, &session);
        if out.close {
            break;
        }
    }
});
