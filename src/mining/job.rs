use crate::types::{JobId, WorkerId, WorkerName};
use anyhow::Result;
use starcoin_consensus::difficult_to_target;
use starcoin_crypto::HashValue;
use starcoin_mining_pool::stratum_rpc::{ShareRequest, StratumJob, StratumJobResponse};
use starcoin_mining_pool::target_hex_to_difficulty;
use starcoin_types::block::BlockHeaderExtra;
use starcoin_types::genesis_config::ConsensusStrategy;
use starcoin_types::U256;

#[derive(Clone, Debug)]
pub struct MiningJob {
    pub worker_id: WorkerId,
    pub worker_name: WorkerName,
    pub job_id: JobId,
    #[allow(dead_code)]
    pub height: u64,
    pub blob: Vec<u8>,
    pub extra: BlockHeaderExtra,
    #[allow(dead_code)]
    pub difficulty: U256,
    pub share_target: U256,
    pub strategy: ConsensusStrategy,
}

impl MiningJob {
    pub fn from_response(
        resp: &StratumJobResponse,
        worker_name: &WorkerName,
        strategy: ConsensusStrategy,
    ) -> Result<Self> {
        Self::from_stratum_job(&resp.id, worker_name, &resp.job, strategy)
    }

    pub fn from_stratum_job(
        worker_id: &str,
        worker_name: &WorkerName,
        job: &StratumJob,
        strategy: ConsensusStrategy,
    ) -> Result<Self> {
        let blob = hex::decode(&job.blob)?;
        let extra = job.get_extra()?;
        let difficulty = target_hex_to_difficulty(&job.target)?;
        let share_target = difficult_to_target(difficulty)?;
        Ok(Self {
            worker_id: WorkerId::parse(worker_id.to_owned())?,
            worker_name: worker_name.clone(),
            job_id: JobId::parse(job.job_id.clone())?,
            height: job.height,
            blob,
            extra,
            difficulty,
            share_target,
            strategy,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SolvedShare {
    pub worker_id: WorkerId,
    pub worker_name: WorkerName,
    pub job_id: JobId,
    pub nonce: u32,
    pub hash: HashValue,
}

impl SolvedShare {
    pub fn into_request(self) -> ShareRequest {
        ShareRequest {
            id: self.worker_id.to_string(),
            job_id: self.job_id.to_string(),
            nonce: crate::mining::pow::nonce_to_hex(self.nonce),
            result: crate::mining::pow::hash_to_result_hex(&self.hash),
        }
    }
}
