//! Top-level writes retain BEGIN IMMEDIATE; nested use cases release only their savepoint.
use rusqlite::{Connection, Savepoint, Transaction, TransactionBehavior};
use std::ops::Deref;
pub(super) enum WriteTransaction<'a> {
    Top(Transaction<'a>),
    Nested(Savepoint<'a>),
}
impl Deref for WriteTransaction<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        match self {
            Self::Top(tx) => tx,
            Self::Nested(tx) => tx,
        }
    }
}
impl WriteTransaction<'_> {
    pub fn commit(self) -> rusqlite::Result<()> {
        match self {
            Self::Top(tx) => tx.commit(),
            Self::Nested(tx) => tx.commit(),
        }
    }
}
pub(super) trait WriteConnection {
    fn write_transaction(&mut self) -> rusqlite::Result<WriteTransaction<'_>>;
}
impl WriteConnection for Connection {
    fn write_transaction(&mut self) -> rusqlite::Result<WriteTransaction<'_>> {
        if self.is_autocommit() {
            self.transaction_with_behavior(TransactionBehavior::Immediate)
                .map(WriteTransaction::Top)
        } else {
            self.savepoint_with_name(format!("omm_{}", uuid::Uuid::new_v4().simple()))
                .map(WriteTransaction::Nested)
        }
    }
}
