//! SQL for one fixed principal, without a wire session.
//!
//! A front door that is not the PostgreSQL protocol (the MCP tool backend of issue #526) still has
//! to run statements as the authenticated principal and nobody else. [`PrincipalSql`] is a
//! [`QueryHandler`] proxy with the principal fixed at creation: its
//! only method takes SQL and parameters, never an identity, so a caller of this type cannot name a
//! different principal, and nothing a client sends can change it. The statement runs on the
//! executor actor under the same row policy, author id, statement ceiling and memory budget as a
//! statement of a PostgreSQL connection of that principal ([`crate::exec_host`]).
//!
//! The principal itself comes from the credential check ([`crate::bearer::verify_bearer`] or a
//! SCRAM exchange); this module does not create one.

use crate::cancel::CancellationToken;
use crate::exec_host::{ExecHost, ExecProxy};
use crate::principal::{Principal, QueryContext};
use crate::session::{QueryError, QueryHandler, QueryOutput};

/// A statement runner bound to one principal. Statements outside a transaction block run one by
/// one, each committing on its own; the runner never opens a block.
pub struct PrincipalSql {
    proxy: ExecProxy,
    principal: Principal,
}

impl PrincipalSql {
    /// The principal every statement of this runner runs as.
    pub fn principal(&self) -> &Principal {
        &self.principal
    }

    /// Runs one statement with text-format parameters (`None` is `NULL`), waiting at most the
    /// host's statement ceiling plus slack.
    pub fn run(
        &mut self,
        sql: &str,
        params: &[Option<Vec<u8>>],
    ) -> Result<QueryOutput, QueryError> {
        self.run_cancellable(sql, params, &CancellationToken::default())
    }

    /// Like [`PrincipalSql::run`], stopping between rows once `cancellation` is cancelled.
    pub fn run_cancellable(
        &mut self,
        sql: &str,
        params: &[Option<Vec<u8>>],
        cancellation: &CancellationToken,
    ) -> Result<QueryOutput, QueryError> {
        self.proxy.execute_cancellable(
            &QueryContext::new(&self.principal),
            sql,
            params,
            cancellation,
        )
    }
}

impl ExecHost {
    /// A statement runner for `principal`. Cheap; its session on the actor starts with its first
    /// statement and ends when the runner is dropped.
    pub fn sql_for(&self, principal: Principal) -> PrincipalSql {
        PrincipalSql {
            proxy: self.handler(),
            principal,
        }
    }
}
