//! The transaction typestate. A change set moves through these states in order; each transition
//! consumes the previous state, so skipping or repeating a step does not compile. A transaction is
//! also bound to the kernel instance that created it, so it cannot be promoted by another kernel.

use crate::changeset::{ChangeSet, ChangeSetId, EncodeError};
use crate::hlc::HlcError;
use crate::log::LogError;
use lattice_abi::{Classify, ErrorClass};
use std::marker::PhantomData;

/// Checks that a change set is well formed against the schema (LSL, from Phase 1 WP7).
pub trait Validator {
    /// `Err(reason)` rejects the change set.
    fn validate(&self, cs: &ChangeSet) -> Result<(), String>;
}

/// Decides whether the author may make this change (the real evaluator arrives in Phase 4).
pub trait Policy {
    /// `Err(reason)` denies the change set.
    fn authorize(&self, cs: &ChangeSet) -> Result<(), String>;
}

/// Accepts everything. Phase 1 placeholder for both traits.
#[derive(Debug, Default, Clone, Copy)]
pub struct AllowAll;

impl Validator for AllowAll {
    fn validate(&self, _cs: &ChangeSet) -> Result<(), String> {
        Ok(())
    }
}

impl Policy for AllowAll {
    fn authorize(&self, _cs: &ChangeSet) -> Result<(), String> {
        Ok(())
    }
}

/// Why a transaction step failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TxnError {
    /// The validator refused the change set.
    #[error("invalid change set: {0}")]
    Invalid(String),
    /// The policy denied the change set.
    #[error("denied: {0}")]
    Denied(String),
    /// The write belongs to a turn that a newer turn has superseded.
    #[error("turn {turn} of session {session} is fenced by turn {current}")]
    Fenced {
        /// Session of the stale write.
        session: u64,
        /// Turn of the stale write.
        turn: u64,
        /// The current turn of the session.
        current: u64,
    },
    /// The write names a turn that was never begun.
    #[error("session {0} has no such turn")]
    UnknownTurn(u64),
    /// A new turn must be greater than the current one.
    #[error("turn {turn} is not newer than the current turn {current}")]
    StaleTurn {
        /// The offered turn.
        turn: u64,
        /// The current turn.
        current: u64,
    },
    /// The id was committed after the transaction was prepared.
    #[error("change set {0} is already committed")]
    Duplicate(ChangeSetId),
    /// The id is queued in an unsynced batch, so its outcome is not known yet.
    #[error("change set {0} is in flight")]
    InFlight(ChangeSetId),
    /// The transaction was created by a different kernel instance (or before a recovery).
    #[error("transaction belongs to a different kernel instance")]
    ForeignTransaction,
    /// A sync returned without the transaction's record being durable.
    #[error("log position {0} is not durable")]
    NotDurable(u64),
    /// The change set cannot be encoded in the format.
    #[error(transparent)]
    Encode(#[from] EncodeError),
    /// The clock refused to issue a timestamp.
    #[error(transparent)]
    Clock(#[from] HlcError),
    /// The log failed.
    #[error(transparent)]
    Log(#[from] LogError),
}

impl Classify for TxnError {
    fn class(&self) -> ErrorClass {
        match self {
            TxnError::Invalid(_)
            | TxnError::Denied(_)
            | TxnError::Fenced { .. }
            | TxnError::UnknownTurn(_)
            | TxnError::StaleTurn { .. }
            | TxnError::Duplicate(_)
            | TxnError::ForeignTransaction
            | TxnError::Encode(_) => ErrorClass::Rejected,
            TxnError::InFlight(_) => ErrorClass::Retryable,
            TxnError::NotDurable(_) => ErrorClass::InvariantViolation,
            TxnError::Clock(e) => e.class(),
            TxnError::Log(e) => e.class(),
        }
    }
}

macro_rules! states {
    ($($(#[$m:meta])* $name:ident),*) => { $( $(#[$m])* #[derive(Debug)] pub struct $name; )* };
}

states! {
    /// Freshly created.
    Open,
    /// Passed schema validation (proof of invariant I8).
    Validated,
    /// Passed the policy check (proof for invariant I7).
    Authorized,
    /// Checked for duplicates and fencing.
    Prepared,
    /// Queued in the write-ahead log with its final timestamp, not yet durable.
    Logged,
    /// Durable: safe to acknowledge.
    Durable,
    /// Reflected in in-memory state.
    Applied
}

/// A change set in state `S`. Fields are private, so a `Txn` in a late state can only come from
/// the kernel having done the earlier work.
#[derive(Debug)]
pub struct Txn<S> {
    pub(crate) cs: ChangeSet,
    pub(crate) lsn: u64,
    pub(crate) owner: u64,
    _state: PhantomData<S>,
}

impl<S> Txn<S> {
    pub(crate) fn new(cs: ChangeSet, lsn: u64, owner: u64) -> Self {
        Self {
            cs,
            lsn,
            owner,
            _state: PhantomData,
        }
    }

    /// The change set.
    pub fn change_set(&self) -> &ChangeSet {
        &self.cs
    }

    /// The change set id.
    pub fn id(&self) -> ChangeSetId {
        self.cs.id
    }
}

impl Txn<Open> {
    pub(crate) fn open(cs: ChangeSet, owner: u64) -> Self {
        Self::new(cs, 0, owner)
    }

    /// Validates against the schema.
    pub fn validate(self, v: &dyn Validator) -> Result<Txn<Validated>, TxnError> {
        v.validate(&self.cs).map_err(TxnError::Invalid)?;
        Ok(Txn::new(self.cs, self.lsn, self.owner))
    }
}

impl Txn<Validated> {
    /// Checks the policy.
    pub fn authorize(self, p: &dyn Policy) -> Result<Txn<Authorized>, TxnError> {
        p.authorize(&self.cs).map_err(TxnError::Denied)?;
        Ok(Txn::new(self.cs, self.lsn, self.owner))
    }
}

/// Result of preparing a transaction.
#[derive(Debug)]
pub enum PrepareOutcome {
    /// Ready to log.
    Ready(Txn<Prepared>),
    /// This id was already committed; nothing will be written.
    Duplicate(ChangeSetId),
}

impl Txn<Authorized> {
    pub(crate) fn into_parts(self) -> (ChangeSet, u64) {
        (self.cs, self.owner)
    }
}

impl Txn<Prepared> {
    pub(crate) fn prepared(cs: ChangeSet, owner: u64) -> Self {
        Self::new(cs, 0, owner)
    }

    pub(crate) fn logged(self, cs: ChangeSet, lsn: u64) -> Txn<Logged> {
        Txn::new(cs, lsn, self.owner)
    }
}

impl Txn<Logged> {
    pub(crate) fn durable(self) -> Txn<Durable> {
        Txn::new(self.cs, self.lsn, self.owner)
    }
}

impl Txn<Durable> {
    pub(crate) fn applied(self) -> Txn<Applied> {
        Txn::new(self.cs, self.lsn, self.owner)
    }
}

impl Txn<Applied> {
    /// The LSN the change set was logged at.
    pub fn lsn(&self) -> u64 {
        self.lsn
    }
}
