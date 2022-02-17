use cosmwasm_std::{
    attr, entry_point, from_binary, to_binary, Addr, Attribute, Binary, Deps, DepsMut, Env,
    MessageInfo, Order, Response, StdError, StdResult, Uint128,
};

use crate::error::ContractError;
use crate::state::{
    Config, CHECKPOINT_TOKEN, CONFIG, OWNERSHIP_PROPOSAL, TIME_CURSOR_OF, TOKENS_PER_WEEK,
};

use crate::utils::{increase_amount_at_week_cursor, transfer_token_amount};
use astroport::asset::addr_validate_to_lower;
use astroport::common::{claim_ownership, drop_ownership_proposal, propose_new_owner};
use astroport::querier::query_token_balance;
use astroport_governance::escrow_fee_distributor::{
    ConfigResponse, Cw20HookMsg, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg,
};
use astroport_governance::utils::{
    get_period, MAX_LIMIT_OF_CLAIM, TOKEN_CHECKPOINT_DEADLINE, WEEK,
};

use astroport_governance::voting_escrow::{
    LockInfoResponse, QueryMsg as VotingQueryMsg, VotingPowerResponse,
};
use cw20::Cw20ReceiveMsg;

use cw2::set_contract_version;
use cw_storage_plus::{Bound, U64Key};

/// Contract name that is used for migration.
const CONTRACT_NAME: &str = "astroport-escrow_fee_distributor";
/// Contract version that is used for migration.
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// ## Description
/// Creates a new contract with the specified parameters in the [`InstantiateMsg`].
/// Returns the default [`Response`] object if the operation was successful, otherwise returns
/// the [`StdResult`] if the contract was not created.
/// ## Params
/// * **msg** is a message of type [`InstantiateMsg`] which contains the basic settings for
/// creating a contract
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let t = msg.start_time / WEEK * WEEK; // week alignment

    CONFIG.save(
        deps.storage,
        &Config {
            owner: addr_validate_to_lower(deps.api, &msg.owner)?,
            astro_token: addr_validate_to_lower(deps.api, &msg.astro_token)?,
            voting_escrow_addr: addr_validate_to_lower(deps.api, &msg.voting_escrow_addr)?,
            emergency_return_addr: addr_validate_to_lower(deps.api, &msg.emergency_return_addr)?,
            start_time: t,
            last_token_time: t,
            time_cursor: t,
            checkpoint_token_enabled: false,
            max_limit_accounts_of_claim: MAX_LIMIT_OF_CLAIM,
            token_last_balance: Uint128::new(0),
        },
    )?;

    Ok(Response::new())
}

/// ## Description
/// Available the execute messages of the contract.
///
/// ## Params
/// * **deps** is the object of type [`Deps`].
///
/// * **env** is the object of type [`Env`].
///
/// * **info** is the object of type [`MessageInfo`].
///
/// * **msg** is the object of type [`ExecuteMsg`].
///
/// ## Queries
/// * **ExecuteMsg::ProposeNewOwner { owner, expires_in }** Creates a request to change ownership.
///
/// * **ExecuteMsg::DropOwnershipProposal {}** Removes a request to change ownership.
///
/// * **ExecuteMsg::ClaimOwnership {}** Approves ownership.
///
/// * **ExecuteMsg::CheckpointTotalSupply {}** Update the vxAstro total supply checkpoint.
///
/// * **ExecuteMsg::Burn { token_address }** Receive tokens into the contract and trigger a token
/// checkpoint.
///
/// * **ExecuteMsg::KillMe {}** Kill the contract. Killing transfers the entire token balance to
/// the emergency return address and blocks the ability to claim or burn. The contract cannot be
/// unkilled.
///
/// * **ExecuteMsg::RecoverBalance { token_address }** Recover tokens from this contract,
/// tokens are sent to the emergency return address.
///
/// * **ExecuteMsg::ToggleAllowCheckpointToken {}** Enables or disables the ability to set
/// a checkpoint token.
///
/// * **ExecuteMsg::Claim { recipient }** Claims the tokens from distributor for transfer
/// to the recipient.
///
/// * **ExecuteMsg::ClaimMany { receivers }**  Make multiple fee claims in a single call.
///
/// * **ExecuteMsg::CheckpointToken {}** Calculates the total number of tokens to be distributed
/// in a given week.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    let config: Config = CONFIG.load(deps.storage)?;
    match msg {
        ExecuteMsg::ProposeNewOwner { owner, expires_in } => propose_new_owner(
            deps,
            info,
            env,
            owner,
            expires_in,
            config.owner,
            OWNERSHIP_PROPOSAL,
        )
        .map_err(|e| e.into()),
        ExecuteMsg::DropOwnershipProposal {} => {
            drop_ownership_proposal(deps, info, config.owner, OWNERSHIP_PROPOSAL)
                .map_err(|e| e.into())
        }
        ExecuteMsg::ClaimOwnership {} => {
            claim_ownership(deps, info, env, OWNERSHIP_PROPOSAL, |deps, new_owner| {
                CONFIG.update::<_, StdError>(deps.storage, |mut v| {
                    v.owner = new_owner;
                    Ok(v)
                })?;

                Ok(())
            })
            .map_err(|e| e.into())
        }
        ExecuteMsg::Claim { recipient } => claim(deps, env, info, recipient),
        ExecuteMsg::ClaimMany { receivers } => claim_many(deps, env, receivers),
        ExecuteMsg::UpdateConfig {
            max_limit_accounts_of_claim,
            checkpoint_token_enabled,
        } => update_config(
            deps,
            info,
            max_limit_accounts_of_claim,
            checkpoint_token_enabled,
        ),
        ExecuteMsg::Receive(msg) => receive_cw20(deps, env, info, msg),
    }
}

/// ## Description
/// Receives a message of type [`Cw20ReceiveMsg`] and processes it depending on the received template.
/// If the template is not found in the received message, then an [`ContractError`] is returned,
/// otherwise returns the [`Response`] with the specified attributes if the operation was successful
fn receive_cw20(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    cw20_msg: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    match from_binary(&cw20_msg.msg)? {
        Cw20HookMsg::Burn {} => {
            let mut config: Config = CONFIG.load(deps.storage)?;
            if info.sender != config.astro_token {
                return Err(ContractError::Unauthorized {});
            }

            // TODO: increment amount for current period in TOKENS_PER_WEEK

            Ok(Response::new())
        }
    }
}

/// ## Description
/// Claims the amount from FeeDistributor for transfer to the recipient. Returns the [`Response`] with
/// specified attributes if operation was successful, otherwise returns the [`ContractError`].
/// ## Params
/// * **recipient** Sets the recipient for claim.
pub fn claim(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    recipient: Option<String>,
) -> Result<Response, ContractError> {
    let recipient_addr = addr_validate_to_lower(
        deps.api,
        &recipient.unwrap_or_else(|| info.sender.to_string()),
    )?;

    let mut config: Config = CONFIG.load(deps.storage)?;

    // TODO: check LAST_CLAIM_PERIOD for user, if no - get from vx contract
    // TODO: iterate for last_period -> current_period - 1 and calculate total reward
    // TODO: send total reward to user
    // TODO: increment LAST_CLAIM_PERIOD for user


    let last_token_time = config.last_token_time / WEEK * WEEK; // week alignment

    let claim_amount = calc_claim_amount(
        deps.branch(),
        config.clone(),
        recipient_addr.clone(),
        last_token_time,
    )?;

    let mut transfer_msg = vec![];
    if !claim_amount.is_zero() {
        transfer_msg = transfer_token_amount(
            config.astro_token.clone(),
            recipient_addr.clone(),
            claim_amount,
        )?;
        config.token_last_balance -= claim_amount;
    };

    CONFIG.save(deps.storage, &config)?;

    let response = Response::new()
        .add_attributes(vec![
            attr("action", "claim"),
            attr("address", recipient_addr.to_string()),
            attr("amount", claim_amount.to_string()),
        ])
        .add_messages(transfer_msg);

    Ok(response)
}

/// ## Description
/// Make multiple fee claims in a single call. Returns the [`Response`] with
/// specified attributes if operation was successful, otherwise returns the [`ContractError`].
/// ## Params
/// * **receivers** is vector field of type [`String`]. Sets the receivers for claim.
fn claim_many(
    mut deps: DepsMut,
    env: Env,
    receivers: Vec<String>,
) -> Result<Response, ContractError> {
    let mut config: Config = CONFIG.load(deps.storage)?;

    if receivers.len() > config.max_limit_accounts_of_claim as usize {
        return Err(ContractError::ExceededAccountLimitOfClaim {});
    }

    try_calc_checkpoint_token(deps.branch(), env.clone(), &mut config)?;

    let last_token_time = config.last_token_time / WEEK * WEEK; // week alignment

    let mut total = Uint128::zero();
    let mut transfer_msg = vec![];

    for receiver in receivers {
        let receiver_addr = addr_validate_to_lower(deps.api, &receiver)?;
        let claim_amount = calc_claim_amount(
            deps.branch(),
            config.clone(),
            receiver_addr.clone(),
            last_token_time,
        )?;

        if !claim_amount.is_zero() {
            transfer_msg.extend(transfer_token_amount(
                config.astro_token.clone(),
                receiver_addr,
                claim_amount,
            )?);
            total += claim_amount;
        };
    }

    if !total.is_zero()
        || (config.checkpoint_token_enabled
            && (env.block.time.seconds() > config.last_token_time + TOKEN_CHECKPOINT_DEADLINE))
    {
        config.token_last_balance -= total;
        CONFIG.save(deps.storage, &config)?;
    }

    let response = Response::new()
        .add_attributes(vec![
            attr("action", "claim_many"),
            attr("amount", total.to_string()),
        ])
        .add_messages(transfer_msg);

    Ok(response)
}

fn try_calc_checkpoint_token(mut deps: DepsMut, env: Env, config: &mut Config) -> StdResult<()> {
    if config.checkpoint_token_enabled
        && (env.block.time.seconds() > config.last_token_time + TOKEN_CHECKPOINT_DEADLINE)
    {
        calc_checkpoint_token(deps.branch(), env, config)?;
    }

    Ok(())
}

/// ## Description
/// Calculation amount of claim.
fn calc_claim_amount(
    deps: DepsMut,
    config: Config,
    addr: Addr,
    last_token_time: u64,
) -> StdResult<Uint128> {
    let user_lock_info: LockInfoResponse = deps.querier.query_wasm_smart(
        &config.voting_escrow_addr,
        &VotingQueryMsg::LockInfo {
            user: addr.to_string(),
        },
    )?;

    let mut week_cursor = TIME_CURSOR_OF
        .may_load(deps.storage, addr.clone())?
        .unwrap_or(config.start_time);

    if week_cursor >= last_token_time {
        return Ok(Uint128::zero());
    }

    let mut claim_amount: Uint128 = Default::default();
    loop {
        if week_cursor >= last_token_time {
            break;
        }

        let current_period = get_period(week_cursor);
        if current_period >= user_lock_info.end {
            break;
        }

        let user_voting_power: VotingPowerResponse = deps.querier.query_wasm_smart(
            &config.voting_escrow_addr,
            &VotingQueryMsg::UserVotingPowerAt {
                user: addr.to_string(),
                time: week_cursor,
            },
        )?;

        let total_voting_power: VotingPowerResponse = deps.querier.query_wasm_smart(
            &config.voting_escrow_addr,
            &VotingQueryMsg::TotalVotingPowerAt { time: week_cursor },
        )?;

        if user_voting_power.voting_power > Uint128::zero() {
            if let Some(tokens_per_week) =
                TOKENS_PER_WEEK.may_load(deps.storage, U64Key::from(current_period))?
            {
                claim_amount = claim_amount.checked_add(
                    user_voting_power
                        .voting_power
                        .checked_mul(tokens_per_week)?
                        .checked_div(total_voting_power.voting_power)?,
                )?;
            }
        }

        week_cursor += WEEK;
    }

    TIME_CURSOR_OF.save(deps.storage, addr, &week_cursor)?;

    Ok(claim_amount)
}

/// ## Description
/// Updates general settings. Returns an [`ContractError`] on failure or the following [`Config`]
/// data will be updated if successful.
fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    max_limit_accounts_of_claim: Option<u64>,
    checkpoint_token_enabled: Option<bool>,
) -> Result<Response, ContractError> {
    let mut attributes = vec![attr("action", "update_config")];
    let mut config: Config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    if let Some(checkpoint_token_enabled) = checkpoint_token_enabled {
        config.checkpoint_token_enabled = checkpoint_token_enabled;
        attributes.push(Attribute::new(
            "checkpoint_token_enabled",
            checkpoint_token_enabled.to_string(),
        ));
    };

    if let Some(max_limit_accounts_of_claim) = max_limit_accounts_of_claim {
        config.max_limit_accounts_of_claim = max_limit_accounts_of_claim;
        attributes.push(Attribute::new(
            "max_limit_accounts_of_claim",
            max_limit_accounts_of_claim.to_string(),
        ));
    };

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new().add_attributes(attributes))
}

/// ## Description
/// Available the query messages of the contract.
/// ## Queries
/// * **QueryMsg::Config {}** Returns the base controls configs that contains in the [`Config`] object.
///
/// * **QueryMsg::AstroRecipientsPerWeek {}** Returns the list of accounts who will get ASTRO fees
/// every week in the [`RecipientsPerWeekResponse`] object.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::FetchUserBalanceByTimestamp { user, timestamp } => {
            to_binary(&query_user_balance(deps, env, user, timestamp)?)
        }
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::VotingSupplyPerWeek { start_after, limit } => {
            to_binary(&query_per_week(deps, start_after, limit)?)
        }
        QueryMsg::FeeTokensPerWeek { start_after, limit } => {
            to_binary(&query_fee_per_week(deps, start_after, limit)?)
        }
    }
}

//settings for pagination
/// The maximum limit for reading pairs from a [`PAIRS`]
const MAX_LIMIT: u64 = 30;

/// The default limit for reading pairs from a [`PAIRS`]
const DEFAULT_LIMIT: u64 = 10;

fn query_per_week(
    deps: Deps,
    start_after: Option<u64>,
    limit: Option<u64>,
) -> StdResult<Vec<Uint128>> {
    let config = CONFIG.load(deps.storage)?;
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let mut start_after = start_after.unwrap_or(config.time_cursor);

    let mut result: Vec<Uint128> = vec![];
    for _i in 0..limit {
        let total_voting_power: VotingPowerResponse = deps.querier.query_wasm_smart(
            &config.voting_escrow_addr,
            &VotingQueryMsg::TotalVotingPowerAt { time: start_after },
        )?;
        start_after += WEEK;
        result.push(total_voting_power.voting_power);
    }

    Ok(result)
}

fn query_fee_per_week(
    deps: Deps,
    start_after: Option<u64>,
    limit: Option<u64>,
) -> StdResult<Vec<Uint128>> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let start;
    if let Some(start_after) = start_after {
        start = Some(Bound::Exclusive(U64Key::from(start_after).wrapped));
    } else {
        start = None;
    }

    Ok(TOKENS_PER_WEEK
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|item| {
            let (_, week_value) = item.unwrap();
            week_value
        })
        .collect())
}
/// ## Description
/// Returns the user fee amount by the timestamp
fn query_user_balance(deps: Deps, _env: Env, user: String, timestamp: u64) -> StdResult<Uint128> {
    let config = CONFIG.load(deps.storage)?;
    let user_voting_power: VotingPowerResponse = deps.querier.query_wasm_smart(
        &config.voting_escrow_addr,
        &VotingQueryMsg::UserVotingPowerAt {
            user,
            time: timestamp,
        },
    )?;

    let total_voting_power: VotingPowerResponse = deps.querier.query_wasm_smart(
        &config.voting_escrow_addr,
        &VotingQueryMsg::TotalVotingPowerAt { time: timestamp },
    )?;

    let mut user_fee_balance = Uint128::zero();
    let current_period = get_period(timestamp);

    if let Some(tokens_per_week) =
        TOKENS_PER_WEEK.may_load(deps.storage, U64Key::from(current_period))?
    {
        user_fee_balance = user_fee_balance.checked_add(
            user_voting_power
                .voting_power
                .checked_mul(tokens_per_week)?
                .checked_div(total_voting_power.voting_power)?,
        )?;
    }

    Ok(user_fee_balance)
}

/// ## Description
/// Returns information about the vesting configs in the [`ConfigResponse`] object.
pub fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config: Config = CONFIG.load(deps.storage)?;

    let resp = ConfigResponse {
        owner: config.owner,
        astro_token: config.astro_token,
        voting_escrow_addr: config.voting_escrow_addr,
        emergency_return_addr: config.emergency_return_addr,
        start_time: config.start_time,
        last_token_time: config.last_token_time,
        time_cursor: config.time_cursor,
        checkpoint_token_enabled: config.checkpoint_token_enabled,
        max_limit_accounts_of_claim: config.max_limit_accounts_of_claim,
    };

    Ok(resp)
}

/// ## Description
/// Used for migration of contract. Returns the default object of type [`Response`].
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::default())
}
