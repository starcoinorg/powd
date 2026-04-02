use std::fmt::{Display, Formatter};
use std::str::FromStr;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct WalletAddress(String);

impl WalletAddress {
    pub fn parse(value: impl Into<String>) -> Result<Self, ParseStratumLoginError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ParseStratumLoginError::MissingWalletAddress);
        }
        Ok(Self(value))
    }
}

impl Display for WalletAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct WorkerName(String);

impl WorkerName {
    pub fn parse(value: impl Into<String>) -> Result<Self, ParseWorkerNameError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ParseWorkerNameError::Empty);
        }
        Ok(Self(value))
    }
}

impl Display for WorkerName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct WorkerId(String);

impl WorkerId {
    pub fn parse(value: impl Into<String>) -> Result<Self, ParseWorkerIdError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ParseWorkerIdError::Empty);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for WorkerId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct JobId(String);

impl JobId {
    pub fn parse(value: impl Into<String>) -> Result<Self, ParseJobIdError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ParseJobIdError::Empty);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for JobId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StratumLogin {
    wallet_address: WalletAddress,
    worker_name: WorkerName,
}

impl StratumLogin {
    pub fn new(wallet_address: WalletAddress, worker_name: WorkerName) -> Self {
        Self {
            wallet_address,
            worker_name,
        }
    }

    pub fn worker_name(&self) -> &WorkerName {
        &self.worker_name
    }
}

impl Display for StratumLogin {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.wallet_address, self.worker_name)
    }
}

impl FromStr for StratumLogin {
    type Err = ParseStratumLoginError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (wallet, worker) = value
            .split_once('.')
            .ok_or(ParseStratumLoginError::MissingSeparator)?;
        let wallet_address = WalletAddress::parse(wallet)?;
        let worker_name = WorkerName::parse(worker)?;
        Ok(Self::new(wallet_address, worker_name))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseWorkerNameError {
    Empty,
}

impl Display for ParseWorkerNameError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("worker_name must not be empty"),
        }
    }
}

impl std::error::Error for ParseWorkerNameError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseJobIdError {
    Empty,
}

impl Display for ParseJobIdError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("job_id must not be empty"),
        }
    }
}

impl std::error::Error for ParseJobIdError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseWorkerIdError {
    Empty,
}

impl Display for ParseWorkerIdError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("worker_id must not be empty"),
        }
    }
}

impl std::error::Error for ParseWorkerIdError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseStratumLoginError {
    MissingSeparator,
    MissingWalletAddress,
    MissingWorkerName,
}

impl Display for ParseStratumLoginError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingSeparator => {
                f.write_str("stratum login must use wallet_address.worker_name")
            }
            Self::MissingWalletAddress => f.write_str("wallet_address must not be empty"),
            Self::MissingWorkerName => f.write_str("worker_name must not be empty"),
        }
    }
}

impl std::error::Error for ParseStratumLoginError {}

impl From<ParseWorkerNameError> for ParseStratumLoginError {
    fn from(value: ParseWorkerNameError) -> Self {
        match value {
            ParseWorkerNameError::Empty => Self::MissingWorkerName,
        }
    }
}
