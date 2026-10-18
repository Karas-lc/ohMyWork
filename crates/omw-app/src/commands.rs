use crate::document_session::{
    CloseAction, CloseDecision, DocumentSaveState, SaveStatus, close_action,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentCommand {
    Save,
    Close(CloseDecision),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandOutcome {
    Noop,
    SaveRequested,
    Close,
    KeepOpen,
}

pub fn reduce_document_command(
    state: &DocumentSaveState,
    command: DocumentCommand,
) -> CommandOutcome {
    match command {
        DocumentCommand::Save => {
            if matches!(
                state.status(),
                SaveStatus::Dirty | SaveStatus::Failed { .. }
            ) {
                CommandOutcome::SaveRequested
            } else {
                CommandOutcome::Noop
            }
        }
        DocumentCommand::Close(decision) => match close_action(state, decision) {
            CloseAction::Close => CommandOutcome::Close,
            CloseAction::RequestSave => CommandOutcome::SaveRequested,
            CloseAction::KeepOpen => CommandOutcome::KeepOpen,
        },
    }
}

pub fn reduce_batch_close(
    states: &[DocumentSaveState],
    decision: CloseDecision,
) -> Vec<CommandOutcome> {
    states
        .iter()
        .map(|state| reduce_document_command(state, DocumentCommand::Close(decision)))
        .collect()
}
