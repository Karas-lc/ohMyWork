use super::SourceRope;

pub struct PageSnapshot {
    revision: u64,
    source: SourceRope,
    external_epoch: u64,
}

impl PageSnapshot {
    pub(crate) fn new(source: SourceRope, revision: u64, external_epoch: u64) -> Self {
        Self { revision, source, external_epoch }
    }

    pub fn revision(&self) -> u64 { self.revision }
    pub fn source(&self) -> &SourceRope { &self.source }
    pub fn external_epoch(&self) -> u64 { self.external_epoch }
}
