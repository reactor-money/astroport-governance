use cosmwasm_std::StdError;
use cw20_base::ContractError as cw20baseError;
use thiserror::Error;

/// ## Description
/// This enum describes vxASTRO contract errors!
#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Cw20Base(#[from] cw20baseError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("Lock already exists")]
    LockAlreadyExists {},

    #[error("Lock does not exist")]
    LockDoesntExist {},

    #[error("Lock time must be within limits (week <= lock time < 2 years)")]
    LockTimeLimitsError {},

    #[error("The lock time has not yet expired")]
    LockHasNotExpired {},

    #[error("The lock expired. Withdraw and create new lock")]
    LockExpired {},

    #[error("The {0} address is blacklisted")]
    AddressBlacklisted(String),

    #[error("Slashed fund receiver is not set yet. Early withdrawal is not available")]
    EarlyWithdrawNotAvailable {},

    #[error("Can not migrate the contract")]
    MigrationError {},
}
