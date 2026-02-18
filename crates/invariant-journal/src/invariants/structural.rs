use invariant_types::JournalEntry;

use crate::error::JournalViolation;

use super::InvariantState;

pub(crate) fn check(_state: &InvariantState, _entry: &JournalEntry) -> Result<(), JournalViolation> {
    Ok(())
}
