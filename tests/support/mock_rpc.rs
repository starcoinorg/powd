#![allow(dead_code)]

use super::process::pick_free_port;
use anyhow::{Context, Result};
use jsonrpc_core::{Error as JsonRpcError, IoHandler, Params};
use jsonrpc_ws_server::{CloseHandle as WsCloseHandle, ServerBuilder};
use serde_json::json;
use starcoin_types::genesis_config::ConsensusStrategy;
use starcoin_types::system_events::MintBlockEvent;
use starcoin_types::U256;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex as StdMutex};

#[derive(Clone)]
pub struct SubmitCall {
    pub minting_blob: String,
    pub _nonce: u32,
    pub extra: String,
}

#[derive(Clone)]
struct MockRpcState {
    current_job: MintBlockEvent,
    submits: Vec<SubmitCall>,
}

pub struct MockMiningRpc {
    state: Arc<StdMutex<MockRpcState>>,
    close_handle: Option<WsCloseHandle>,
    wait_thread: Option<std::thread::JoinHandle<()>>,
    addr: SocketAddr,
}

impl MockMiningRpc {
    pub fn start(initial_job: MintBlockEvent) -> Result<Self> {
        let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), pick_free_port()?);
        let state = Arc::new(StdMutex::new(MockRpcState {
            current_job: initial_job,
            submits: Vec::new(),
        }));

        let mut io = IoHandler::default();
        add_job_methods(&mut io, &state);
        add_chain_methods(&mut io, &state);

        let server = ServerBuilder::new(io)
            .start(&addr)
            .context("start mock mining rpc ws server failed")?;

        let close_handle = server.close_handle();
        let wait_thread = std::thread::spawn(move || {
            let _ = server.wait();
        });

        Ok(Self {
            state,
            close_handle: Some(close_handle),
            wait_thread: Some(wait_thread),
            addr,
        })
    }

    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    pub fn submit_calls(&self) -> Result<Vec<SubmitCall>> {
        let guard = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("mock rpc mutex poisoned"))?;
        Ok(guard.submits.clone())
    }
}

impl Drop for MockMiningRpc {
    fn drop(&mut self) {
        let close_handle = self.close_handle.take();
        let wait_thread = self.wait_thread.take();
        if close_handle.is_some() || wait_thread.is_some() {
            std::thread::spawn(move || {
                if let Some(close_handle) = close_handle {
                    close_handle.close();
                }
                if let Some(wait_thread) = wait_thread {
                    let _ = wait_thread.join();
                }
            });
        }
    }
}

pub fn build_mint_event(
    number: u64,
    difficulty: u64,
    strategy: ConsensusStrategy,
) -> MintBlockEvent {
    let mut minting_blob = vec![0u8; 76];
    minting_blob[0..8].copy_from_slice(&number.to_le_bytes());
    minting_blob[8..16].copy_from_slice(&difficulty.to_le_bytes());
    MintBlockEvent::new(
        starcoin_crypto::HashValue::random(),
        strategy,
        minting_blob,
        U256::from(difficulty.max(1)),
        number,
        None,
    )
}

fn add_job_methods(io: &mut IoHandler, state: &Arc<StdMutex<MockRpcState>>) {
    let state_for_get_job = Arc::clone(state);
    io.add_sync_method("mining.get_job", move |_params: Params| {
        let guard = state_for_get_job
            .lock()
            .map_err(|_| JsonRpcError::internal_error())?;
        serde_json::to_value(Some(guard.current_job.clone()))
            .map_err(|_| JsonRpcError::internal_error())
    });

    let state_for_submit = Arc::clone(state);
    io.add_sync_method("mining.submit", move |params: Params| {
        let (minting_blob, nonce, extra): (String, u32, String) = params
            .parse()
            .map_err(|_| JsonRpcError::invalid_params("invalid submit params"))?;
        let mut guard = state_for_submit
            .lock()
            .map_err(|_| JsonRpcError::internal_error())?;
        guard.submits.push(SubmitCall {
            minting_blob,
            _nonce: nonce,
            extra,
        });
        let block_hash = guard.current_job.parent_hash;
        Ok(json!({ "block_hash": block_hash }))
    });
}

fn add_chain_methods(io: &mut IoHandler, state: &Arc<StdMutex<MockRpcState>>) {
    io.add_sync_method("chain.info", move |_params: Params| {
        Ok(json!({
            "chain_id": 255,
            "genesis_hash": format!("0x{}", "88".repeat(32)),
            "head": mock_header_json(1),
            "block_info": {
                "block_hash": format!("0x{}", "99".repeat(32)),
                "total_difficulty": "0x01",
                "txn_accumulator_info": mock_accumulator_info_json(),
                "block_accumulator_info": mock_accumulator_info_json()
            }
        }))
    });

    let state_for_node_info = Arc::clone(state);
    io.add_sync_method("node.info", move |_params: Params| {
        let consensus = {
            let guard = state_for_node_info
                .lock()
                .map_err(|_| JsonRpcError::internal_error())?;
            guard.current_job.strategy
        };
        Ok(json!({
            "peer_info": {
                "peer_id": "12D3KooW9yQoKZrByqrUjmmPHXtR23qCXRQvF5KowYgoqypuhuCn",
                "chain_info": {
                    "chain_id": 255,
                    "genesis_hash": format!("0x{}", "88".repeat(32)),
                    "head": mock_header_json(1),
                    "block_info": {
                        "block_hash": format!("0x{}", "99".repeat(32)),
                        "total_difficulty": "0x01",
                        "txn_accumulator_info": mock_accumulator_info_json(),
                        "block_accumulator_info": mock_accumulator_info_json()
                    }
                },
                "notif_protocols": "",
                "rpc_protocols": "",
                "version_string": null
            },
            "self_address": "/ip4/127.0.0.1/tcp/0",
            "net": "main",
            "consensus": {"type": consensus_type_name(consensus)},
            "now_seconds": 0
        }))
    });

    io.add_sync_method("chain.get_block_by_number", move |params: Params| {
        let (number, _option): (u64, Option<serde_json::Value>) = params
            .parse()
            .map_err(|_| JsonRpcError::invalid_params("invalid get_block_by_number params"))?;
        Ok(json!({
            "header": mock_header_json(number),
            "body": {"Hashes": []},
            "uncles": [],
            "raw": serde_json::Value::Null
        }))
    });
    io.add_sync_method("chain.get_block_txn_infos", move |_params: Params| {
        Ok(json!([]))
    });
    io.add_sync_method("chain.get_events_by_txn_hash", move |_params: Params| {
        Ok(json!([]))
    });
}

fn consensus_type_name(strategy: ConsensusStrategy) -> &'static str {
    match strategy {
        ConsensusStrategy::Dummy => "Dummy",
        ConsensusStrategy::Argon => "Argon",
        ConsensusStrategy::Keccak => "Keccak",
        ConsensusStrategy::CryptoNight => "CryptoNight",
    }
}

fn mock_header_json(number: u64) -> serde_json::Value {
    json!({
        "block_hash": format!("0x{}", "11".repeat(32)),
        "parent_hash": format!("0x{}", "22".repeat(32)),
        "timestamp": "0",
        "number": number.to_string(),
        "author": "0x00000000000000000000000000000001",
        "author_auth_key": serde_json::Value::Null,
        "txn_accumulator_root": format!("0x{}", "33".repeat(32)),
        "block_accumulator_root": format!("0x{}", "44".repeat(32)),
        "state_root": format!("0x{}", "55".repeat(32)),
        "gas_used": "0",
        "difficulty": "0x01",
        "body_hash": format!("0x{}", "66".repeat(32)),
        "chain_id": 255,
        "nonce": 0,
        "extra": "0x00000000",
        "parents_hash": [],
        "version": 0,
        "pruning_point": format!("0x{}", "77".repeat(32))
    })
}

fn mock_accumulator_info_json() -> serde_json::Value {
    json!({
        "accumulator_root": format!("0x{}", "88".repeat(32)),
        "frozen_subtree_roots": [],
        "num_leaves": "0",
        "num_nodes": "0"
    })
}
