use astroport::asset::addr_validate_to_lower;
use astroport::common::{claim_ownership, drop_ownership_proposal, propose_new_owner};
#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_binary, to_binary, Addr, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult, Uint128, WasmMsg,
};
use cw2::{get_contract_version, set_contract_version};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg};

use crate::contract::helpers::compute_unlocked_amount;
use crate::migration::{MigrateMsg, CONFIGV100, STATEV100, STATUSV100};
use astroport_governance::builder_unlock::msg::{
    AllocationResponse, ExecuteMsg, InstantiateMsg, QueryMsg, ReceiveMsg, SimulateWithdrawResponse,
    StateResponse,
};
use astroport_governance::builder_unlock::{AllocationParams, AllocationStatus, Config, State};

use crate::state::{CONFIG, OWNERSHIP_PROPOSAL, PARAMS, STATE, STATUS};

// Version and name used for contract migration.
const CONTRACT_NAME: &str = "builder-unlock";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// ## Description
/// Creates a new contract with the specified parameters in the `msg` variable.
/// Returns a [`Response`] with the specified attributes if the operation was successful,
/// or a [`ContractError`] if the contract was not created.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **_env** is an object of type [`Env`]
///
/// * **_info** is an object of type [`MessageInfo`]
///
/// * **msg**  is a message of type [`InstantiateMsg`] which contains the parameters used for creating a contract.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(
        deps.storage,
        &Config {
            owner: deps.api.addr_validate(&msg.owner)?,
            rct_token: deps.api.addr_validate(&msg.rct_token)?,
            max_allocations_amount: msg.max_allocations_amount,
        },
    )?;
    Ok(Response::default())
}

/// ## Description
/// Exposes all the execute functions available in the contract.
///
/// ## Execute messages
/// * **ExecuteMsg::Receive(cw20_msg)** Parse incoming messages coming from the RCT token contract.
///
/// * **ExecuteMsg::Withdraw** Withdraw unlocked RCT.
///
/// * **ExecuteMsg::TransferOwnership** Transfer contract ownership.
///
/// * **ExecuteMsg::ProposeNewReceiver** Propose a new receiver for a specific RCT unlock schedule.
///
/// * **ExecuteMsg::DropNewReceiver** Drop the proposal to change the receiver for an unlock schedule.
///
/// * **ExecuteMsg::ClaimReceiver**  Claim the position as a receiver for a specific unlock schedule.
///
/// * **ExecuteMsg::IncreaseAllocation** Increase RCT allocation for receiver.
///
/// * **ExecuteMsg::DecreaseAllocation** Decrease RCT allocation for receiver.
///
/// * **ExecuteMsg::TransferUnallocated** Transfer unallocated tokens.
///
/// * **ExecuteMsg::ProposeNewOwner** Creates a new request to change contract ownership.
///
/// * **ExecuteMsg::DropOwnershipProposal** Removes a request to change contract ownership.
///
/// * **ExecuteMsg::ClaimOwnership** Claims contract ownership.
///
/// * **ExecuteMsg::UpdateConfig** Update contract configuration.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(deps: DepsMut, env: Env, info: MessageInfo, msg: ExecuteMsg) -> StdResult<Response> {
    match msg {
        ExecuteMsg::Receive(cw20_msg) => execute_receive_cw20(deps, info, cw20_msg),
        ExecuteMsg::Withdraw {} => execute_withdraw(deps, env, info),
        ExecuteMsg::ProposeNewReceiver { new_receiver } => {
            execute_propose_new_receiver(deps, info, new_receiver)
        }
        ExecuteMsg::DropNewReceiver {} => execute_drop_new_receiver(deps, info),
        ExecuteMsg::ClaimReceiver { prev_receiver } => {
            execute_claim_receiver(deps, info, prev_receiver)
        }
        ExecuteMsg::IncreaseAllocation { receiver, amount } => {
            let config = CONFIG.load(deps.storage)?;
            if info.sender != config.owner {
                return Err(StdError::generic_err(
                    "Only the contract owner can increase allocations",
                ));
            }
            execute_increase_allocation(deps, &config, receiver, amount, None)
        }
        ExecuteMsg::DecreaseAllocation { receiver, amount } => {
            execute_decrease_allocation(deps, env, info, receiver, amount)
        }
        ExecuteMsg::TransferUnallocated { amount, recipient } => {
            execute_transfer_unallocated(deps, info, amount, recipient)
        }
        ExecuteMsg::ProposeNewOwner {
            new_owner,
            expires_in,
        } => {
            let config: Config = CONFIG.load(deps.storage)?;

            propose_new_owner(
                deps,
                info,
                env,
                new_owner,
                expires_in,
                config.owner,
                OWNERSHIP_PROPOSAL,
            )
        }
        ExecuteMsg::DropOwnershipProposal {} => {
            let config: Config = CONFIG.load(deps.storage)?;

            drop_ownership_proposal(deps, info, config.owner, OWNERSHIP_PROPOSAL)
        }
        ExecuteMsg::ClaimOwnership {} => {
            claim_ownership(deps, info, env, OWNERSHIP_PROPOSAL, |deps, new_owner| {
                CONFIG.update::<_, StdError>(deps.storage, |mut v| {
                    v.owner = new_owner;
                    Ok(v)
                })?;

                Ok(())
            })
        }
        ExecuteMsg::UpdateConfig {
            new_max_allocations_amount,
        } => update_config(deps, info, new_max_allocations_amount),
    }
}

/// ## Description
/// Receives a message of type [`Cw20ReceiveMsg`] and processes it depending on the received template.
/// If the template is not found in the received message, then a [`ContractError`] is returned,
/// otherwise it returns a [`Response`] with the specified attributes if the operation was successful.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **info** is an object of type [`MessageInfo`].
///
/// * **cw20_msg** is an object of type [`Cw20ReceiveMsg`]. This is the CW20 message to process.
fn execute_receive_cw20(
    deps: DepsMut,
    info: MessageInfo,
    cw20_msg: Cw20ReceiveMsg,
) -> StdResult<Response> {
    match from_binary(&cw20_msg.msg)? {
        ReceiveMsg::CreateAllocations { allocations } => execute_create_allocations(
            deps,
            cw20_msg.sender,
            info.sender,
            cw20_msg.amount,
            allocations,
        ),
        ReceiveMsg::IncreaseAllocation { user, amount } => {
            let config = CONFIG.load(deps.storage)?;

            if config.rct_token != info.sender {
                return Err(StdError::generic_err("Only RCT can be deposited"));
            }
            if addr_validate_to_lower(deps.api, &cw20_msg.sender)? != config.owner {
                return Err(StdError::generic_err(
                    "Only the contract owner can increase allocations",
                ));
            }

            execute_increase_allocation(deps, &config, user, amount, Some(cw20_msg.amount))
        }
    }
}

/// ## Description
/// Expose available contract queries.
/// ## Params
/// * **deps** is an object of type [`Deps`].
///
/// * **_env** is an object of type [`Env`].
///
/// * **msg** is an object of type [`QueryMsg`].
///
/// ## Queries
/// * **QueryMsg::Config {}** Return the contract configuration.
///
/// * **QueryMsg::State {}** Return the contract state (number of RCT that still need to be withdrawn).
///
/// * **QueryMsg::Allocation {}** Return the allocation details for a specific account.
///
/// * **QueryMsg::UnlockedTokens {}** Return the amoint of unlocked RCT for a specific account.
///
/// * **QueryMsg::SimulateWithdraw {}** Return the result of a withdrawal simulation.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::State {} => to_binary(&query_state(deps)?),
        QueryMsg::Allocation { account } => to_binary(&query_allocation(deps, account)?),
        QueryMsg::UnlockedTokens { account } => {
            to_binary(&query_tokens_unlocked(deps, env, account)?)
        }
        QueryMsg::SimulateWithdraw { account, timestamp } => {
            to_binary(&query_simulate_withdraw(deps, env, account, timestamp)?)
        }
    }
}

/// ## Description
/// Admin function facilitating creation of new allocations.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **creator** is an object of type [`String`]. This is the allocations creator (the contract admin).
///
/// * **deposit_token** is an object of type [`Addr`]. This is the token being deposited (should be RCT).
///
/// * **deposit_amount** is an object of type [`Uint128`]. This is the of tokens sent along with the call (should equal the sum of allocation amounts)
///
/// * **deposit_amount** is a vector of tuples of type [(`String`, `AllocationParams`)]. New allocations being created.
fn execute_create_allocations(
    deps: DepsMut,
    creator: String,
    deposit_token: Addr,
    deposit_amount: Uint128,
    allocations: Vec<(String, AllocationParams)>,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.may_load(deps.storage)?.unwrap_or_default();

    if deps.api.addr_validate(&creator)? != config.owner {
        return Err(StdError::generic_err(
            "Only the contract owner can create allocations",
        ));
    }

    if deposit_token != config.rct_token {
        return Err(StdError::generic_err("Only RCT can be deposited"));
    }

    if deposit_amount != allocations.iter().map(|params| params.1.amount).sum() {
        return Err(StdError::generic_err("RCT deposit amount mismatch"));
    }

    state.total_rct_deposited += deposit_amount;
    state.remaining_rct_tokens += deposit_amount;

    if state.total_rct_deposited > config.max_allocations_amount {
        return Err(StdError::generic_err(format!(
            "The total allocation for all recipients cannot exceed total RCT amount allocated to unlock (currently {} RCT)",
            config.max_allocations_amount,
        )));
    }

    for allocation in allocations {
        let (user_unchecked, params) = allocation;

        let user = deps.api.addr_validate(&user_unchecked)?;

        match PARAMS.load(deps.storage, &user) {
            Ok(..) => {
                return Err(StdError::generic_err(format!(
                    "Allocation (params) already exists for {}",
                    user
                )));
            }
            Err(..) => {
                PARAMS.save(deps.storage, &user, &params)?;
            }
        }

        match STATUS.load(deps.storage, &user) {
            Ok(..) => {
                return Err(StdError::generic_err(format!(
                    "Allocation (status) already exists for {}",
                    user
                )));
            }
            Err(..) => {
                STATUS.save(deps.storage, &user, &AllocationStatus::new())?;
            }
        }
    }

    STATE.save(deps.storage, &state)?;
    Ok(Response::default())
}

/// ## Description
/// Allow allocation recipients to withdraw unlocked RCT.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **env** is an object of type [`Env`].
///
/// * **info** is an object of type [`MessageInfo`].
fn execute_withdraw(deps: DepsMut, env: Env, info: MessageInfo) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.may_load(deps.storage)?.unwrap_or_default();

    let params = PARAMS.load(deps.storage, &info.sender)?;
    let mut status = STATUS.load(deps.storage, &info.sender)?;

    let SimulateWithdrawResponse { rct_to_withdraw: reactor_to_withdraw } =
        helpers::compute_withdraw_amount(env.block.time.seconds(), &params, &mut status);

    state.remaining_rct_tokens -= reactor_to_withdraw;

    // SAVE :: state & allocation
    STATE.save(deps.storage, &state)?;

    // Update status
    STATUS.save(deps.storage, &info.sender, &status)?;

    let mut msgs: Vec<WasmMsg> = vec![];

    if reactor_to_withdraw.is_zero() {
        return Err(StdError::generic_err("No unlocked RCT to be withdrawn"));
    }

    msgs.push(WasmMsg::Execute {
        contract_addr: config.rct_token.to_string(),
        msg: to_binary(&Cw20ExecuteMsg::Transfer {
            recipient: info.sender.to_string(),
            amount: reactor_to_withdraw,
        })?,
        funds: vec![],
    });

    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("rct_withdrawn", reactor_to_withdraw))
}

/// ## Description
/// Allows the current allocation receiver to propose a new receiver/.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **info** is an object of type [`MessageInfo`].
///
/// * **new_receiver** is an object of type [`String`]. Newly proposed receiver for the allocation.
fn execute_propose_new_receiver(
    deps: DepsMut,
    info: MessageInfo,
    new_receiver: String,
) -> StdResult<Response> {
    let mut alloc_params = PARAMS.load(deps.storage, &info.sender)?;

    match alloc_params.proposed_receiver {
        Some(proposed_receiver) => {
            return Err(StdError::generic_err(format!(
                "Proposed receiver already set to {}",
                proposed_receiver
            )));
        }
        None => {
            let alloc_params_new_receiver = PARAMS
                .may_load(deps.storage, &deps.api.addr_validate(&new_receiver)?)?
                .unwrap_or_default();
            if !alloc_params_new_receiver.amount.is_zero() {
                return Err(StdError::generic_err(format!(
                    "Invalid new_receiver. Proposed receiver already has an RCT allocation of {} RCT",
                    alloc_params_new_receiver.amount
                )));
            }

            alloc_params.proposed_receiver = Some(deps.api.addr_validate(&new_receiver)?);
            PARAMS.save(deps.storage, &info.sender, &alloc_params)?;
        }
    }

    Ok(Response::new()
        .add_attribute("action", "ProposeNewReceiver")
        .add_attribute("proposed_receiver", new_receiver))
}

/// ## Description
/// Drop the newly proposed receiver for a specific allocation.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **info** is an object of type [`MessageInfo`].
fn execute_drop_new_receiver(deps: DepsMut, info: MessageInfo) -> StdResult<Response> {
    let mut alloc_params = PARAMS.load(deps.storage, &info.sender)?;
    let prev_proposed_receiver: Addr;

    match alloc_params.proposed_receiver {
        Some(proposed_receiver) => {
            prev_proposed_receiver = proposed_receiver;
            alloc_params.proposed_receiver = None;
            PARAMS.save(deps.storage, &info.sender, &alloc_params)?;
        }
        None => {
            return Err(StdError::generic_err("Proposed receiver not set"));
        }
    }

    Ok(Response::new()
        .add_attribute("action", "DropNewReceiver")
        .add_attribute("dropped_proposed_receiver", prev_proposed_receiver))
}

/// ## Description
/// Decrease allocation.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **env** is an object of type [`Env`].
///
/// * **info** is an object of type [`MessageInfo`].
///
/// * **receiver** is an object of type [`String`]. Decreasing receiver.
///
/// * **amount** is an object of type [`Uint128`]. RCT amount to decrease.
fn execute_decrease_allocation(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    receiver: String,
    amount: Uint128,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(StdError::generic_err(
            "Only the contract owner can decrease allocations",
        ));
    }

    let receiver = addr_validate_to_lower(deps.api, &receiver)?;

    let mut state = STATE.load(deps.storage)?;
    let mut params = PARAMS.load(deps.storage, &receiver)?;
    let mut status = STATUS.load(deps.storage, &receiver)?;

    let unlocked_amount = compute_unlocked_amount(
        env.block.time.seconds(),
        params.amount,
        &params.unlock_schedule,
        status.unlocked_amount_checkpoint,
    );
    let locked_amount = params.amount - unlocked_amount;

    if locked_amount < amount {
        return Err(StdError::generic_err(format!(
            "Insufficient amount of lock to decrease allocation, User has locked {} RCT.",
            locked_amount
        )));
    }

    params.amount = params.amount.checked_sub(amount)?;
    status.unlocked_amount_checkpoint = unlocked_amount;
    state.unallocated_tokens = state.unallocated_tokens.checked_add(amount)?;
    state.remaining_rct_tokens = state.remaining_rct_tokens.checked_sub(amount)?;

    STATUS.save(deps.storage, &receiver, &status)?;
    PARAMS.save(deps.storage, &receiver, &params)?;
    STATE.save(deps.storage, &state)?;

    Ok(Response::new()
        .add_attribute("action", "execute_decrease_allocation")
        .add_attribute("receiver", receiver)
        .add_attribute("amount", amount))
}

/// ## Description
/// Increase allocation.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **config** is an object of type [`Config`].
///
/// * **receiver** is an object of type [`String`]. Increasing receiver.
///
/// * **amount** is an object of type [`Uint128`]. RCT amount to increase.
///
/// * **deposit_amount** is an [`Option`] of type [`Uint128`]. Amount of RCT to increase using CW20 Receive.
fn execute_increase_allocation(
    deps: DepsMut,
    config: &Config,
    receiver: String,
    amount: Uint128,
    deposit_amount: Option<Uint128>,
) -> StdResult<Response> {
    let receiver = addr_validate_to_lower(deps.api, &receiver)?;

    match PARAMS.may_load(deps.storage, &receiver)? {
        Some(mut params) => {
            let mut state = STATE.load(deps.storage)?;

            if let Some(deposit_amount) = deposit_amount {
                state.total_rct_deposited =
                    state.total_rct_deposited.checked_add(deposit_amount)?;
                state.unallocated_tokens = state.unallocated_tokens.checked_add(deposit_amount)?;

                if state.total_rct_deposited > config.max_allocations_amount {
                    return Err(StdError::generic_err(format!(
                        "The total allocation for all recipients cannot exceed total RCT amount allocated to unlock (currently {} RCT)",
                        config.max_allocations_amount,
                    )));
                }
            }

            if state.unallocated_tokens < amount {
                return Err(StdError::generic_err(format!(
                    "Insufficient unallocated RCT to increase allocation. Contract has: {} unallocated RCT.",
                    state.unallocated_tokens
                )));
            }

            params.amount = params.amount.checked_add(amount)?;
            state.unallocated_tokens = state.unallocated_tokens.checked_sub(amount)?;
            state.remaining_rct_tokens = state.remaining_rct_tokens.checked_add(amount)?;

            PARAMS.save(deps.storage, &receiver, &params)?;
            STATE.save(deps.storage, &state)?;
        }
        None => {
            return Err(StdError::generic_err("Proposed receiver not set"));
        }
    }

    Ok(Response::new()
        .add_attribute("action", "execute_increase_allocation")
        .add_attribute("amount", amount)
        .add_attribute("receiver", receiver))
}

/// ## Description
/// Transfer unallocated RCT tokens to recipient.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **info** is an object of type [`MessageInfo`].
///
/// * **amount** is an object of type [`Uint128`]. Amount RCT to transfer.
///
/// * **recipient** is an [`Option`] of type [`u64`]. Transfer recipient.
fn execute_transfer_unallocated(
    deps: DepsMut,
    info: MessageInfo,
    amount: Uint128,
    recipient: Option<String>,
) -> StdResult<Response> {
    let recipient = match recipient {
        Some(addr) => addr_validate_to_lower(deps.api, &addr)?,
        None => info.sender.clone(),
    };

    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;

    if config.owner != info.sender {
        return Err(StdError::generic_err(
            "Only contract owner can transfer unallocated RCT.",
        ));
    }

    if state.unallocated_tokens < amount {
        return Err(StdError::generic_err(format!(
            "Insufficient unallocated RCT to transfer. Contract has: {} unallocated RCT.",
            state.unallocated_tokens
        )));
    }

    state.unallocated_tokens = state.unallocated_tokens.checked_sub(amount)?;

    let msg = WasmMsg::Execute {
        contract_addr: config.rct_token.to_string(),
        msg: to_binary(&Cw20ExecuteMsg::Transfer {
            recipient: recipient.to_string(),
            amount,
        })?,
        funds: vec![],
    };

    STATE.save(deps.storage, &state)?;

    Ok(Response::new()
        .add_attribute("action", "execute_transfer_unallocated")
        .add_attribute("amount", amount)
        .add_message(msg))
}

/// ## Description
/// Allows a newly proposed allocation receiver to claim the ownership of that allocation.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **info** is an object of type [`MessageInfo`].
///
/// * **prev_receiver** is an object of type [`String`]. This is the previous receiver for hte allocation.
fn execute_claim_receiver(
    deps: DepsMut,
    info: MessageInfo,
    prev_receiver: String,
) -> StdResult<Response> {
    let mut alloc_params = PARAMS.load(deps.storage, &deps.api.addr_validate(&prev_receiver)?)?;

    match alloc_params.proposed_receiver {
        Some(proposed_receiver) => {
            if proposed_receiver == info.sender {
                if let Some(sender_params) = PARAMS.may_load(deps.storage, &info.sender)? {
                    return Err(StdError::generic_err(format!(
                        "The proposed receiver already has an RCT allocation of {} RCT, that ends at {}",
                        sender_params.amount,
                        sender_params.unlock_schedule.start_time + sender_params.unlock_schedule.duration + sender_params.unlock_schedule.cliff,
                    )));
                }

                // Transfers Allocation Parameters ::
                // 1. Save the allocation for the new receiver
                alloc_params.proposed_receiver = None;

                PARAMS.save(deps.storage, &info.sender, &alloc_params)?;
                // 2. Remove the allocation info from the previous owner
                PARAMS.remove(deps.storage, &deps.api.addr_validate(&prev_receiver)?);
                // Transfers Allocation Status ::
                let mut status =
                    STATUS.load(deps.storage, &deps.api.addr_validate(&prev_receiver)?)?;

                if let Some(sender_status) = STATUS.may_load(deps.storage, &info.sender)? {
                    status.rct_withdrawn = status
                        .rct_withdrawn
                        .checked_add(sender_status.rct_withdrawn)?;
                }

                STATUS.save(deps.storage, &info.sender, &status)?;
                STATUS.remove(deps.storage, &deps.api.addr_validate(&prev_receiver)?)
            } else {
                return Err(StdError::generic_err(format!(
                    "Proposed receiver mismatch, actual proposed receiver : {}",
                    proposed_receiver
                )));
            }
        }
        None => {
            return Err(StdError::generic_err("Proposed receiver not set"));
        }
    }

    Ok(Response::new()
        .add_attribute("action", "ClaimReceiver")
        .add_attribute("prev_receiver", prev_receiver)
        .add_attribute("new_receiver", info.sender.to_string()))
}

/// ## Description
/// Updates contract parameters.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **info** is an object of type [`MessageInfo`].
///
/// * **new_max_allocations_amount** is an object of type [`Uint128`].
fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    new_max_allocations_amount: Uint128,
) -> StdResult<Response> {
    let mut config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(StdError::generic_err(
            "Only the contract owner can change config",
        ));
    }

    config.max_allocations_amount = new_max_allocations_amount;
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new()
        .add_attribute("action", "update_config")
        .add_attribute("new_max_allocations_amount", new_max_allocations_amount))
}

/// ## Description
/// Return the contract configuration.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
fn query_config(deps: Deps) -> StdResult<Config> {
    CONFIG.load(deps.storage)
}

/// ## Description
/// Return the global distribution state.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
pub fn query_state(deps: Deps) -> StdResult<StateResponse> {
    let state = STATE.may_load(deps.storage)?.unwrap_or_default();
    Ok(StateResponse {
        total_rct_deposited: state.total_rct_deposited,
        remaining_rct_tokens: state.remaining_rct_tokens,
        unallocated_rct_tokens: state.unallocated_tokens,
    })
}

/// ## Description
/// Return information about a specific allocation.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **account** is an object of type [`String`]. This is the account whose allocation we query.
fn query_allocation(deps: Deps, account: String) -> StdResult<AllocationResponse> {
    let account_checked = deps.api.addr_validate(&account)?;

    Ok(AllocationResponse {
        params: PARAMS
            .may_load(deps.storage, &account_checked)?
            .unwrap_or_default(),
        status: STATUS
            .may_load(deps.storage, &account_checked)?
            .unwrap_or_default(),
    })
}

/// ## Description
/// Return the total amount of unlocked tokens for a specific account.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **env** is an object of type [`Env`].
///
/// * **account** is an object of type [`String`]. This is the account whose unlocked token amount we query.
fn query_tokens_unlocked(deps: Deps, env: Env, account: String) -> StdResult<Uint128> {
    let account_checked = deps.api.addr_validate(&account)?;

    let params = PARAMS.load(deps.storage, &account_checked)?;
    let status = STATUS.load(deps.storage, &account_checked)?;

    Ok(helpers::compute_unlocked_amount(
        env.block.time.seconds(),
        params.amount,
        &params.unlock_schedule,
        status.unlocked_amount_checkpoint,
    ))
}

/// ## Description
/// Simulate a token withdrawal.
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **env** is an object of type [`Env`].
///
/// * **account** is an object of type [`String`]. This is the account for which we simulate a withdrawal.
///
/// * **timestamp** is an [`Option`] of type [`u64`]. This is the timestamp where we assume the account would withdraw.
fn query_simulate_withdraw(
    deps: Deps,
    env: Env,
    account: String,
    timestamp: Option<u64>,
) -> StdResult<SimulateWithdrawResponse> {
    let account_checked = deps.api.addr_validate(&account)?;

    let params = PARAMS.load(deps.storage, &account_checked)?;
    let mut status = STATUS.load(deps.storage, &account_checked)?;

    let timestamp_ = match timestamp {
        Some(timestamp) => timestamp,
        None => env.block.time.seconds(),
    };

    Ok(helpers::compute_withdraw_amount(
        timestamp_,
        &params,
        &mut status,
    ))
}

/// ## Description
/// Used for contract migration. Returns a default object of type [`Response`].
/// ## Params
/// * **deps** is an object of type [`DepsMut`].
///
/// * **_env** is an object of type [`Env`].
///
/// * **msg** is an object of type [`Empty`].
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, msg: MigrateMsg) -> StdResult<Response> {
    let contract_version = get_contract_version(deps.storage)?;

    match contract_version.contract.as_ref() {
        "builder-unlock" => match contract_version.version.as_ref() {
            "1.0.0" => {
                let state_v100 = STATEV100.load(deps.storage)?;
                STATE.save(
                    deps.storage,
                    &State {
                        total_rct_deposited: state_v100.total_rct_deposited,
                        remaining_rct_tokens: state_v100.remaining_rct_tokens,
                        unallocated_tokens: Uint128::zero(),
                    },
                )?;

                let keys = STATUSV100
                    .keys(deps.storage, None, None, cosmwasm_std::Order::Ascending {})
                    .map(|v| String::from_utf8(v).map_err(StdError::from))
                    .collect::<Result<Vec<String>, StdError>>()?;

                for key in keys {
                    let status_v100 = STATUSV100.load(deps.storage, &Addr::unchecked(&key))?;
                    let status = AllocationStatus {
                        rct_withdrawn: status_v100.rct_withdrawn,
                        unlocked_amount_checkpoint: Uint128::zero(),
                    };
                    STATUS.save(deps.storage, &Addr::unchecked(key), &status)?;
                }

                let config_v100 = CONFIGV100.load(deps.storage)?;

                CONFIG.save(
                    deps.storage,
                    &Config {
                        owner: config_v100.owner,
                        rct_token: config_v100.rct_token,
                        max_allocations_amount: msg.max_allocations_amount,
                    },
                )?;
            }
            _ => return Err(StdError::generic_err("Contract can't be migrated!")),
        },
        _ => return Err(StdError::generic_err("Contract can't be migrated!")),
    };

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    Ok(Response::new()
        .add_attribute("previous_contract_name", &contract_version.contract)
        .add_attribute("previous_contract_version", &contract_version.version)
        .add_attribute("new_contract_name", CONTRACT_NAME)
        .add_attribute("new_contract_version", CONTRACT_VERSION))
}

//----------------------------------------------------------------------------------------
// Helper Functions
//----------------------------------------------------------------------------------------

mod helpers {
    use cosmwasm_std::Uint128;

    use astroport_governance::builder_unlock::msg::SimulateWithdrawResponse;
    use astroport_governance::builder_unlock::{AllocationParams, AllocationStatus, Schedule};

    // Computes number of tokens that are now unlocked for a given allocation
    pub fn compute_unlocked_amount(
        timestamp: u64,
        amount: Uint128,
        schedule: &Schedule,
        unlock_checkpoint: Uint128,
    ) -> Uint128 {
        // Tokens haven't begun unlocking
        if timestamp < schedule.start_time + schedule.cliff {
            Uint128::zero()
        }
        // Tokens unlock linearly between start time and end time
        else if (timestamp < schedule.start_time + schedule.cliff + schedule.duration)
            && schedule.duration != 0
        {
            let unlocked_amount = amount.multiply_ratio(
                timestamp - (schedule.start_time + schedule.cliff),
                schedule.duration,
            );

            if unlocked_amount > unlock_checkpoint {
                unlocked_amount
            } else {
                unlock_checkpoint
            }
        }
        // After end time, all tokens are fully unlocked
        else {
            amount
        }
    }

    // Computes number of tokens that are withdrawable for a given allocation
    pub fn compute_withdraw_amount(
        timestamp: u64,
        params: &AllocationParams,
        status: &mut AllocationStatus,
    ) -> SimulateWithdrawResponse {
        // "Unlocked" amount
        let rct_unlocked = compute_unlocked_amount(
            timestamp,
            params.amount,
            &params.unlock_schedule,
            status.unlocked_amount_checkpoint,
        );

        // Withdrawable amount is unlocked amount minus the amount already withdrawn
        let rct_withdrawable = rct_unlocked - status.rct_withdrawn;
        status.rct_withdrawn += rct_withdrawable;

        SimulateWithdrawResponse {
            rct_to_withdraw: rct_withdrawable,
        }
    }
}
