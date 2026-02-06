use damascus_conv::TranscriptRound;
use damascus_types::FileId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("round already exists")]
    DuplicateRound,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InMemoryChain {
    rounds: HashMap<(FileId, u64, u32), TranscriptRound>,
}

impl InMemoryChain {
    pub fn insert(&mut self, round: TranscriptRound) -> Result<(), ChainError> {
        let key = (round.mb_vec.file_id, round.mb_vec.epoch, round.mb_vec.round);
        if self.rounds.contains_key(&key) {
            return Err(ChainError::DuplicateRound);
        }
        self.rounds.insert(key, round);
        Ok(())
    }

    pub fn get(&self, file_id: FileId, epoch: u64, j: u32) -> Option<&TranscriptRound> {
        self.rounds.get(&(file_id, epoch, j))
    }

    pub fn transcript_sorted(&self, file_id: FileId, epoch: u64) -> Vec<TranscriptRound> {
        let mut items: Vec<_> = self
            .rounds
            .iter()
            .filter_map(|((fid, e, _), v)| (*fid == file_id && *e == epoch).then(|| v.clone()))
            .collect();
        items.sort_by_key(|t| t.mb_vec.round);
        items
    }
}
