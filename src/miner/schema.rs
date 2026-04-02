use super::{ControlErrorKind, MinerCapabilities};
use serde::Serialize;
use std::collections::BTreeMap;

pub const CONTROL_PLANE_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ControlPlaneMethods {
    pub control_plane_version: u32,
    pub agent_version: String,
    pub methods: BTreeMap<String, MethodSpec>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MethodSpec {
    pub params: Option<MethodParamsSchema>,
    pub result: String,
    pub errors: Vec<MethodErrorSchema>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MethodParamsSchema {
    pub kind: String,
    pub fields: BTreeMap<String, MethodFieldSchema>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MethodFieldSchema {
    #[serde(rename = "type")]
    pub type_name: String,
    pub optional: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MethodErrorSchema {
    pub code: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<ControlErrorKind>,
    pub description: String,
}

pub fn build_control_plane_methods(capabilities: &MinerCapabilities) -> ControlPlaneMethods {
    let common_runtime_errors = vec![
        MethodErrorSchema {
            code: -32000,
            kind: Some(ControlErrorKind::RuntimeFailed),
            description: "runtime failed while processing request".to_string(),
        },
        MethodErrorSchema {
            code: -32000,
            kind: Some(ControlErrorKind::RuntimeTerminated),
            description: "runtime terminated before request completed".to_string(),
        },
        MethodErrorSchema {
            code: -32000,
            kind: Some(ControlErrorKind::TransitionTimeout),
            description: "runtime state transition timed out".to_string(),
        },
    ];
    let mut methods = BTreeMap::new();
    methods.insert(
        "miner.start".to_string(),
        MethodSpec {
            params: None,
            result: "miner_snapshot".to_string(),
            errors: common_runtime_errors.clone(),
        },
    );
    methods.insert(
        "miner.stop".to_string(),
        MethodSpec {
            params: None,
            result: "miner_snapshot".to_string(),
            errors: common_runtime_errors.clone(),
        },
    );
    methods.insert(
        "miner.pause".to_string(),
        MethodSpec {
            params: None,
            result: "miner_snapshot".to_string(),
            errors: common_runtime_errors.clone(),
        },
    );
    methods.insert(
        "miner.resume".to_string(),
        MethodSpec {
            params: None,
            result: "miner_snapshot".to_string(),
            errors: common_runtime_errors.clone(),
        },
    );
    methods.insert(
        "budget.set_mode".to_string(),
        MethodSpec {
            params: Some(MethodParamsSchema {
                kind: "object".to_string(),
                fields: BTreeMap::from([(
                    "mode".to_string(),
                    MethodFieldSchema {
                        type_name: "string".to_string(),
                        optional: false,
                        minimum: None,
                        maximum: None,
                        enum_values: capabilities
                            .supported_modes
                            .iter()
                            .map(serde_name)
                            .collect(),
                    },
                )]),
            }),
            result: "miner_snapshot".to_string(),
            errors: vec![
                MethodErrorSchema {
                    code: -32602,
                    kind: None,
                    description: "invalid params".to_string(),
                },
                MethodErrorSchema {
                    code: -32000,
                    kind: Some(ControlErrorKind::RuntimeFailed),
                    description: "runtime failed while processing request".to_string(),
                },
                MethodErrorSchema {
                    code: -32000,
                    kind: Some(ControlErrorKind::RuntimeTerminated),
                    description: "runtime terminated before request completed".to_string(),
                },
                MethodErrorSchema {
                    code: -32000,
                    kind: Some(ControlErrorKind::TransitionTimeout),
                    description: "runtime state transition timed out".to_string(),
                },
            ],
        },
    );
    methods.insert(
        "budget.set".to_string(),
        MethodSpec {
            params: Some(MethodParamsSchema {
                kind: "object".to_string(),
                fields: BTreeMap::from([
                    (
                        "threads".to_string(),
                        MethodFieldSchema {
                            type_name: "u16".to_string(),
                            optional: true,
                            minimum: Some(1),
                            maximum: Some(u64::from(capabilities.max_threads)),
                            enum_values: Vec::new(),
                        },
                    ),
                    (
                        "cpu_percent".to_string(),
                        MethodFieldSchema {
                            type_name: "u8".to_string(),
                            optional: true,
                            minimum: Some(1),
                            maximum: Some(100),
                            enum_values: Vec::new(),
                        },
                    ),
                    (
                        "priority".to_string(),
                        MethodFieldSchema {
                            type_name: "string".to_string(),
                            optional: true,
                            minimum: None,
                            maximum: None,
                            enum_values: capabilities
                                .supported_priorities
                                .iter()
                                .map(serde_name)
                                .collect(),
                        },
                    ),
                ]),
            }),
            result: "miner_snapshot".to_string(),
            errors: vec![
                MethodErrorSchema {
                    code: -32602,
                    kind: None,
                    description: "invalid params".to_string(),
                },
                MethodErrorSchema {
                    code: -32000,
                    kind: Some(ControlErrorKind::InvalidBudget),
                    description: "budget is outside supported limits".to_string(),
                },
                MethodErrorSchema {
                    code: -32000,
                    kind: Some(ControlErrorKind::RuntimeFailed),
                    description: "runtime failed while processing request".to_string(),
                },
                MethodErrorSchema {
                    code: -32000,
                    kind: Some(ControlErrorKind::RuntimeTerminated),
                    description: "runtime terminated before request completed".to_string(),
                },
                MethodErrorSchema {
                    code: -32000,
                    kind: Some(ControlErrorKind::TransitionTimeout),
                    description: "runtime state transition timed out".to_string(),
                },
            ],
        },
    );
    methods.insert(
        "status.get".to_string(),
        MethodSpec {
            params: None,
            result: "miner_snapshot".to_string(),
            errors: Vec::new(),
        },
    );
    methods.insert(
        "status.capabilities".to_string(),
        MethodSpec {
            params: None,
            result: "miner_capabilities".to_string(),
            errors: Vec::new(),
        },
    );
    methods.insert(
        "status.methods".to_string(),
        MethodSpec {
            params: None,
            result: "control_plane_methods".to_string(),
            errors: Vec::new(),
        },
    );
    methods.insert(
        "events.since".to_string(),
        MethodSpec {
            params: Some(MethodParamsSchema {
                kind: "object".to_string(),
                fields: BTreeMap::from([(
                    "since_seq".to_string(),
                    MethodFieldSchema {
                        type_name: "u64".to_string(),
                        optional: false,
                        minimum: Some(0),
                        maximum: None,
                        enum_values: Vec::new(),
                    },
                )]),
            }),
            result: "events_since_response".to_string(),
            errors: vec![MethodErrorSchema {
                code: -32602,
                kind: None,
                description: "invalid params".to_string(),
            }],
        },
    );
    methods.insert(
        "events.stream".to_string(),
        MethodSpec {
            params: None,
            result: "subscription_ack".to_string(),
            errors: vec![MethodErrorSchema {
                code: -32000,
                kind: None,
                description: "events already subscribed on this connection".to_string(),
            }],
        },
    );

    ControlPlaneMethods {
        control_plane_version: CONTROL_PLANE_VERSION,
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        methods,
    }
}

pub(crate) fn serde_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .expect("serialize control schema enum")
        .as_str()
        .expect("control schema enum should serialize to string")
        .to_string()
}
