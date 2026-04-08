use super::config::{reward_api_base_url, MintProfile};
use crate::MintNetwork;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::io;
use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct WalletRewardSnapshot {
    pub account: String,
    pub network: MintNetwork,
    pub confirmed_total_raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_pending_total_raw: Option<String>,
    pub paid_total_raw: String,
    pub confirmed_total_display: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_pending_total_display: Option<String>,
    pub paid_total_display: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmed_through_height: Option<u64>,
    pub confirmed_blocks_24h: u64,
    pub orphaned_blocks_24h: u64,
    pub source_base_url: String,
}

#[derive(Debug)]
pub(crate) enum RewardError {
    Http(ureq::Error),
    Read(io::Error),
    Decode(serde_json::Error),
    InvalidResponse(&'static str),
    Join(tokio::task::JoinError),
}

impl Display for RewardError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(err) => write!(f, "reward query failed: {err}"),
            Self::Read(err) => write!(f, "read reward response failed: {err}"),
            Self::Decode(err) => write!(f, "decode reward response failed: {err}"),
            Self::InvalidResponse(reason) => write!(f, "invalid reward response: {reason}"),
            Self::Join(err) => write!(f, "reward query task failed: {err}"),
        }
    }
}

impl std::error::Error for RewardError {}

#[derive(Deserialize)]
struct MiningDashboardResponse {
    account: String,
    summary: MiningSummary,
}

#[derive(Deserialize)]
struct MiningSummary {
    confirmed_blocks_24h: u64,
    orphaned_blocks_24h: u64,
    confirmed_total: String,
    paid_total: String,
    confirmed_through_height: Option<u64>,
    estimated_pending_total: Option<String>,
}

pub(crate) async fn fetch_wallet_reward(
    profile: &MintProfile,
    timeout: Duration,
) -> Result<WalletRewardSnapshot, RewardError> {
    let profile = profile.clone();
    tokio::task::spawn_blocking(move || fetch_wallet_reward_blocking(&profile, timeout))
        .await
        .map_err(RewardError::Join)?
}

fn fetch_wallet_reward_blocking(
    profile: &MintProfile,
    timeout: Duration,
) -> Result<WalletRewardSnapshot, RewardError> {
    let base_url = reward_api_base_url(profile.network);
    let url = format!(
        "{}/v1/mining/dashboard/{}?window_secs=300",
        base_url, profile.wallet_address
    );
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let response = agent.get(&url).call().map_err(RewardError::Http)?;
    let body = response.into_string().map_err(RewardError::Read)?;
    let payload: MiningDashboardResponse =
        serde_json::from_str(&body).map_err(RewardError::Decode)?;
    build_snapshot(payload, profile.network, base_url)
}

fn build_snapshot(
    payload: MiningDashboardResponse,
    network: MintNetwork,
    source_base_url: String,
) -> Result<WalletRewardSnapshot, RewardError> {
    if payload.account.trim().is_empty() {
        return Err(RewardError::InvalidResponse("missing account"));
    }
    let summary = payload.summary;
    Ok(WalletRewardSnapshot {
        account: payload.account,
        network,
        confirmed_total_raw: summary.confirmed_total.clone(),
        estimated_pending_total_raw: summary.estimated_pending_total.clone(),
        paid_total_raw: summary.paid_total.clone(),
        confirmed_total_display: format_stc_from_nano(&summary.confirmed_total),
        estimated_pending_total_display: summary
            .estimated_pending_total
            .as_deref()
            .map(format_stc_from_nano),
        paid_total_display: format_stc_from_nano(&summary.paid_total),
        confirmed_through_height: summary.confirmed_through_height,
        confirmed_blocks_24h: summary.confirmed_blocks_24h,
        orphaned_blocks_24h: summary.orphaned_blocks_24h,
        source_base_url,
    })
}

pub(crate) fn format_stc_from_nano(raw_value: &str) -> String {
    let raw = raw_value.trim();
    if raw.is_empty() {
        return "0.0 STC".to_string();
    }
    match raw.parse::<i128>() {
        Ok(value) => {
            let negative = value < 0;
            let abs = value.unsigned_abs();
            let rounded_tenths = (abs + 50_000_000) / 100_000_000;
            let whole = rounded_tenths / 10;
            let frac = rounded_tenths % 10;
            let sign = if negative && rounded_tenths > 0 {
                "-"
            } else {
                ""
            };
            format!("{sign}{whole}.{frac} STC")
        }
        Err(_) => format!("{raw} (raw)"),
    }
}

#[cfg(test)]
mod tests {
    use super::format_stc_from_nano;

    #[test]
    fn format_stc_rounds_to_tenths() {
        assert_eq!(format_stc_from_nano("0"), "0.0 STC");
        assert_eq!(format_stc_from_nano("1"), "0.0 STC");
        assert_eq!(format_stc_from_nano("50000000"), "0.1 STC");
        assert_eq!(format_stc_from_nano("1499999999"), "1.5 STC");
        assert_eq!(format_stc_from_nano("1500000000"), "1.5 STC");
        assert_eq!(format_stc_from_nano("-50000000"), "-0.1 STC");
        assert_eq!(format_stc_from_nano("bogus"), "bogus (raw)");
    }
}
