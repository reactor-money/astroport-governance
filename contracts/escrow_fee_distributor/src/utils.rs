use crate::error::ContractError;
use astroport_governance::escrow_fee_distributor::WEEK;
use cosmwasm_std::{to_binary, Addr, CosmosMsg, DepsMut, StdResult, Uint128, WasmMsg};
use cw20::Cw20ExecuteMsg;
use cw_storage_plus::{Map, U64Key};

/// ## Description
/// Transfer amount of token.
pub fn transfer_token_amount(
    contract_addr: Addr,
    recipient: Addr,
    amount: Uint128,
) -> Result<Vec<CosmosMsg>, ContractError> {
    let messages = if !amount.is_zero() {
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: contract_addr.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: recipient.to_string(),
                amount,
            })?,
            funds: vec![],
        })]
    } else {
        vec![]
    };

    Ok(messages)
}

/// ## Description
/// Returns the week number.
pub fn get_period(time: u64) -> u64 {
    time / WEEK
}

/// ## Description
/// Create or update item with specified parameters in the map
pub fn save_or_update_state_config(
    deps: DepsMut,
    config: &Map<U64Key, Uint128>,
    week_cursor: u64,
    amount: Uint128,
) -> StdResult<()> {
    config.update(
        deps.storage,
        U64Key::from(week_cursor),
        |cursor| -> StdResult<_> {
            if let Some(cursor_value) = cursor {
                Ok(cursor_value + amount)
            } else {
                Ok(amount)
            }
        },
    )?;

    Ok(())
}