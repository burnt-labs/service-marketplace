#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    BankMsg, Binary, Coin, Deps, DepsMut, Env, MessageInfo, Order, Response, StdResult, Uint128,
    to_json_binary,
};
use cw_ownable::assert_owner;
use cw_storage_plus::Bound;
use cw_utils::{must_pay, nonpayable};
use cw2::set_contract_version;

use crate::error::ContractError;
use crate::helpers::{assert_creator, next_ask_id, next_bid_id};
use crate::msg::{AskResponse, BidResponse, ExecuteMsg, InstantiateMsg, QueryMsg, RecipientShare};
use crate::state::{ALLOWED_DENOMS, ASKS, Ask, AskStatus, Bid, bids};

const CONTRACT_NAME: &str = "crates.io:service-marketplace";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_RESOLVE_RECIPIENTS: usize = 5;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    cw_ownable::initialize_owner(deps.storage, deps.api, Some(msg.owner.as_str()))?;
    ALLOWED_DENOMS.save(deps.storage, &msg.allowed_denoms)?;
    Ok(Response::new().add_attribute("action", "instantiate"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdateOwnership(action) => {
            let ownership = cw_ownable::update_ownership(deps, &env.block, &info.sender, action)?;
            Ok(Response::new().add_attributes(ownership.into_attributes()))
        }
        ExecuteMsg::CreateAsk { budget, metadata_url } => {
            create_ask(deps, env, info, budget, metadata_url)
        }
        ExecuteMsg::SubmitBid {
            ask_id,
            price,
            funds_recipient,
        } => submit_bid(deps, env, info, ask_id, price, funds_recipient),
        ExecuteMsg::SelectBid {
            ask_id,
            bid_id,
            expected_price,
        } => select_bid(deps, info, ask_id, bid_id, expected_price),
        ExecuteMsg::CancelAsk { ask_id } => cancel_ask(deps, info, ask_id),
        ExecuteMsg::MarkComplete { ask_id } => mark_complete(deps, info, ask_id),
        ExecuteMsg::ConfirmCompletion { ask_id } => confirm_completion(deps, info, ask_id),
        ExecuteMsg::PartialDisbursement { ask_id, amount } => {
            partial_disbursement(deps, info, ask_id, amount)
        }
        ExecuteMsg::AdminCancel { ask_id, refund_to } => {
            admin_cancel(deps, info, ask_id, refund_to)
        }
        ExecuteMsg::AdminForceRelease { ask_id, recipient } => {
            admin_force_release(deps, info, ask_id, recipient)
        }
        ExecuteMsg::AdminResolve { ask_id, splits } => admin_resolve(deps, info, ask_id, splits),
        ExecuteMsg::AdminSetAllowedDenoms { denoms } => {
            admin_set_allowed_denoms(deps, info, denoms)
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Ownership {} => to_json_binary(&cw_ownable::get_ownership(deps.storage)?),
        QueryMsg::GetAsk { ask_id } => to_json_binary(&query_ask(deps, ask_id)?),
        QueryMsg::GetBid { bid_id } => to_json_binary(&query_bid(deps, bid_id)?),
        QueryMsg::ListBidsByAsk {
            ask_id,
            start_after,
            limit,
        } => to_json_binary(&query_bids_by_ask(deps, ask_id, start_after, limit)?),
        QueryMsg::ListAsks { start_after, limit } => {
            to_json_binary(&query_asks(deps, start_after, limit)?)
        }
        QueryMsg::GetAllowedDenoms {} => to_json_binary(&query_allowed_denoms(deps)?),
    }
}

/// Maximum length for metadata_url (2048 chars — standard browser URL limit)
const MAX_METADATA_URL_LEN: usize = 2048;

fn validate_url(url: &str) -> Result<(), ContractError> {
    if url.is_empty() || url.len() > MAX_METADATA_URL_LEN {
        return Err(ContractError::InvalidMetadataUrl {});
    }
    if !url.starts_with("https://") && !url.starts_with("ipfs://") && !url.starts_with("ar://") {
        return Err(ContractError::InvalidMetadataUrl {});
    }
    Ok(())
}

fn create_ask(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    budget: Coin,
    metadata_url: String,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;

    if budget.denom.is_empty() || budget.amount.is_zero() {
        return Err(ContractError::DenomNotAllowed {
            denom: budget.denom,
        });
    }

    let allowed = ALLOWED_DENOMS.load(deps.storage)?;
    if !allowed.is_empty() && !allowed.contains(&budget.denom) {
        return Err(ContractError::DenomNotAllowed {
            denom: budget.denom,
        });
    }

    validate_url(&metadata_url)?;

    let id = next_ask_id(deps.storage, &env, &info)?;
    let ask = Ask {
        id: id.clone(),
        creator: info.sender,
        budget,
        metadata_url,
        status: AskStatus::Open,
        selected_bid: None,
        locked_funds: None,
    };
    ASKS.save(deps.storage, &id, &ask)?;
    Ok(Response::new()
        .add_attribute("action", "create_ask")
        .add_attribute("ask_id", id))
}

fn submit_bid(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    ask_id: String,
    price: Coin,
    funds_recipient: Option<String>,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let ask = ASKS
        .load(deps.storage, &ask_id)
        .map_err(|_| ContractError::AskNotFound { id: ask_id.clone() })?;

    if ask.creator == info.sender {
        return Err(ContractError::CreatorCannotBid {});
    }

    if !matches!(ask.status, AskStatus::Open) {
        return Err(ContractError::InvalidStatus {
            expected: "Open".to_string(),
            got: format!("{:?}", ask.status),
        });
    }

    if price.amount.is_zero() {
        return Err(ContractError::InsufficientFunds {});
    }
    if price.denom != ask.budget.denom {
        return Err(ContractError::BidPriceMismatch {
            expected: ask.budget.denom.clone(),
            sent: price.denom.clone(),
        });
    }

    if bids()
        .idx
        .by_ask_provider
        .item(deps.storage, (ask_id.clone(), info.sender.to_string()))?
        .is_some()
    {
        return Err(ContractError::AlreadyBid {});
    }

    let funds_recipient = funds_recipient
        .map(|r| deps.api.addr_validate(&r))
        .transpose()?;

    let id = next_bid_id(deps.storage, &env, &info)?;
    let bid = Bid {
        id: id.clone(),
        ask_id: ask_id.clone(),
        provider: info.sender.clone(),
        price,
        funds_recipient,
    };
    bids().save(deps.storage, id.clone(), &bid)?;

    Ok(Response::new()
        .add_attribute("action", "submit_bid")
        .add_attribute("ask_id", ask_id)
        .add_attribute("bid_id", id))
}

fn select_bid(
    deps: DepsMut,
    info: MessageInfo,
    ask_id: String,
    bid_id: String,
    expected_price: Coin,
) -> Result<Response, ContractError> {
    let mut ask = ASKS
        .load(deps.storage, &ask_id)
        .map_err(|_| ContractError::AskNotFound { id: ask_id.clone() })?;

    assert_creator(&ask, &info)?;

    if !matches!(ask.status, AskStatus::Open) {
        return Err(ContractError::InvalidStatus {
            expected: "Open".to_string(),
            got: format!("{:?}", ask.status),
        });
    }

    let bid = bids()
        .load(deps.storage, bid_id.clone())
        .map_err(|_| ContractError::BidNotFound { id: bid_id.clone() })?;

    if bid.ask_id != ask_id {
        return Err(ContractError::BidNotFound { id: bid_id });
    }

    // Front-run protection: client commits to the price they saw; reject if it changed
    if expected_price.amount != bid.price.amount || expected_price.denom != bid.price.denom {
        return Err(ContractError::BidPriceMismatch {
            expected: format!("{}{}", bid.price.amount, bid.price.denom),
            sent: format!("{}{}", expected_price.amount, expected_price.denom),
        });
    }

    // Validate sent funds: exactly one coin, correct denom, correct amount
    let sent_amount = must_pay(&info, &bid.price.denom)?;
    if sent_amount != bid.price.amount {
        return Err(ContractError::BidPriceMismatch {
            expected: format!("{}{}", bid.price.amount, bid.price.denom),
            sent: format!("{}{}", sent_amount, bid.price.denom),
        });
    }

    ask.selected_bid = Some(bid_id.clone());
    ask.locked_funds = Some(bid.price.clone());
    ask.status = AskStatus::InProgress;
    ASKS.save(deps.storage, &ask_id, &ask)?;

    Ok(Response::new()
        .add_attribute("action", "select_bid")
        .add_attribute("ask_id", ask_id)
        .add_attribute("bid_id", bid_id))
}

fn cancel_ask(deps: DepsMut, info: MessageInfo, ask_id: String) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let mut ask = ASKS
        .load(deps.storage, &ask_id)
        .map_err(|_| ContractError::AskNotFound { id: ask_id.clone() })?;

    assert_creator(&ask, &info)?;

    if !matches!(ask.status, AskStatus::Open) {
        return Err(ContractError::InvalidStatus {
            expected: "Open".to_string(),
            got: format!("{:?}", ask.status),
        });
    }

    ask.status = AskStatus::Cancelled;
    ASKS.save(deps.storage, &ask_id, &ask)?;

    Ok(Response::new()
        .add_attribute("action", "cancel_ask")
        .add_attribute("ask_id", ask_id))
}

fn mark_complete(
    deps: DepsMut,
    info: MessageInfo,
    ask_id: String,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let mut ask = ASKS
        .load(deps.storage, &ask_id)
        .map_err(|_| ContractError::AskNotFound { id: ask_id.clone() })?;

    if !matches!(ask.status, AskStatus::InProgress) {
        return Err(ContractError::InvalidStatus {
            expected: "InProgress".to_string(),
            got: format!("{:?}", ask.status),
        });
    }

    let bid_id = ask
        .selected_bid
        .as_ref()
        .ok_or(ContractError::NoSelectedBid {})?
        .clone();
    let bid = bids()
        .load(deps.storage, bid_id.clone())
        .map_err(|_| ContractError::BidNotFound { id: bid_id })?;

    if bid.provider != info.sender {
        return Err(ContractError::NotBidProvider {});
    }

    ask.status = AskStatus::WorkSubmitted;
    ASKS.save(deps.storage, &ask_id, &ask)?;

    Ok(Response::new()
        .add_attribute("action", "mark_complete")
        .add_attribute("ask_id", ask_id))
}

fn confirm_completion(
    deps: DepsMut,
    info: MessageInfo,
    ask_id: String,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let mut ask = ASKS
        .load(deps.storage, &ask_id)
        .map_err(|_| ContractError::AskNotFound { id: ask_id.clone() })?;

    assert_creator(&ask, &info)?;

    if !matches!(ask.status, AskStatus::WorkSubmitted) {
        return Err(ContractError::InvalidStatus {
            expected: "WorkSubmitted".to_string(),
            got: format!("{:?}", ask.status),
        });
    }

    let bid_id = ask
        .selected_bid
        .as_ref()
        .ok_or(ContractError::NoSelectedBid {})?
        .clone();
    let bid = bids()
        .load(deps.storage, bid_id.clone())
        .map_err(|_| ContractError::BidNotFound { id: bid_id })?;

    let recipient = bid.funds_recipient.unwrap_or(bid.provider);
    let mut msgs = vec![];
    if let Some(locked) = ask.locked_funds.take() {
        if !locked.amount.is_zero() {
            msgs.push(BankMsg::Send {
                to_address: recipient.to_string(),
                amount: vec![locked],
            });
        }
    }

    ask.status = AskStatus::Completed;
    ASKS.save(deps.storage, &ask_id, &ask)?;

    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "confirm_completion")
        .add_attribute("ask_id", ask_id))
}

fn partial_disbursement(
    deps: DepsMut,
    info: MessageInfo,
    ask_id: String,
    amount: Coin,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    let mut ask = ASKS
        .load(deps.storage, &ask_id)
        .map_err(|_| ContractError::AskNotFound { id: ask_id.clone() })?;

    assert_creator(&ask, &info)?;

    if !matches!(ask.status, AskStatus::InProgress | AskStatus::WorkSubmitted) {
        return Err(ContractError::InvalidStatus {
            expected: "InProgress or WorkSubmitted".to_string(),
            got: format!("{:?}", ask.status),
        });
    }

    let bid_id = ask
        .selected_bid
        .as_ref()
        .ok_or(ContractError::NoSelectedBid {})?
        .clone();
    let bid = bids()
        .load(deps.storage, bid_id.clone())
        .map_err(|_| ContractError::BidNotFound { id: bid_id })?;

    if amount.denom != ask.budget.denom {
        return Err(ContractError::BidPriceMismatch {
            expected: ask.budget.denom.clone(),
            sent: amount.denom.clone(),
        });
    }

    let locked = ask
        .locked_funds
        .as_mut()
        .ok_or(ContractError::InsufficientFunds {})?;

    if amount.amount.is_zero() || amount.amount > locked.amount {
        return Err(ContractError::InsufficientFunds {});
    }

    locked.amount -= amount.amount;
    ASKS.save(deps.storage, &ask_id, &ask)?;

    Ok(Response::new()
        .add_message(BankMsg::Send {
            to_address: bid.payout_address().to_string(),
            amount: vec![amount],
        })
        .add_attribute("action", "partial_disbursement")
        .add_attribute("ask_id", ask_id))
}

fn admin_cancel(
    deps: DepsMut,
    info: MessageInfo,
    ask_id: String,
    refund_to: String,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    assert_owner(deps.storage, &info.sender)?;

    let mut ask = ASKS
        .load(deps.storage, &ask_id)
        .map_err(|_| ContractError::AskNotFound { id: ask_id.clone() })?;

    if matches!(ask.status, AskStatus::Completed | AskStatus::Cancelled) {
        return Err(ContractError::InvalidStatus {
            expected: "any active status".to_string(),
            got: format!("{:?}", ask.status),
        });
    }

    let refund_addr = deps.api.addr_validate(&refund_to)?;
    let mut msgs = vec![];
    if let Some(locked) = ask.locked_funds.take() {
        if !locked.amount.is_zero() {
            msgs.push(BankMsg::Send {
                to_address: refund_addr.to_string(),
                amount: vec![locked],
            });
        }
    }

    ask.status = AskStatus::Cancelled;
    ASKS.save(deps.storage, &ask_id, &ask)?;

    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "admin_cancel")
        .add_attribute("ask_id", ask_id))
}

fn admin_force_release(
    deps: DepsMut,
    info: MessageInfo,
    ask_id: String,
    recipient: String,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    assert_owner(deps.storage, &info.sender)?;

    let mut ask = ASKS
        .load(deps.storage, &ask_id)
        .map_err(|_| ContractError::AskNotFound { id: ask_id.clone() })?;

    if matches!(ask.status, AskStatus::Completed | AskStatus::Cancelled) {
        return Err(ContractError::InvalidStatus {
            expected: "any active status".to_string(),
            got: format!("{:?}", ask.status),
        });
    }

    let recipient_addr = deps.api.addr_validate(&recipient)?;
    let mut msgs = vec![];
    if let Some(locked) = ask.locked_funds.take() {
        if !locked.amount.is_zero() {
            msgs.push(BankMsg::Send {
                to_address: recipient_addr.to_string(),
                amount: vec![locked],
            });
        }
    }

    ask.status = AskStatus::Completed;
    ASKS.save(deps.storage, &ask_id, &ask)?;

    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "admin_force_release")
        .add_attribute("ask_id", ask_id))
}

fn admin_resolve(
    deps: DepsMut,
    info: MessageInfo,
    ask_id: String,
    splits: Vec<RecipientShare>,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    assert_owner(deps.storage, &info.sender)?;

    // Reject zero-amount entries — every split must actually receive funds
    if splits.iter().any(|s| s.amount.is_zero()) {
        return Err(ContractError::InsufficientFunds {});
    }

    if splits.is_empty() || splits.len() > MAX_RESOLVE_RECIPIENTS {
        return Err(ContractError::InvalidRecipientsCount {
            min: 1,
            max: MAX_RESOLVE_RECIPIENTS,
        });
    }

    let mut ask = ASKS
        .load(deps.storage, &ask_id)
        .map_err(|_| ContractError::AskNotFound { id: ask_id.clone() })?;

    if matches!(ask.status, AskStatus::Completed | AskStatus::Cancelled) {
        return Err(ContractError::InvalidStatus {
            expected: "any active status".to_string(),
            got: format!("{:?}", ask.status),
        });
    }

    let locked = ask
        .locked_funds
        .take()
        .ok_or(ContractError::InsufficientFunds {})?;

    // Splits must sum to exactly the locked amount (overflow-safe fold)
    let total = splits
        .iter()
        .try_fold(Uint128::zero(), |acc, s| acc.checked_add(s.amount))
        .map_err(|_| ContractError::InsufficientFunds {})?;

    if total != locked.amount {
        return Err(ContractError::BidPriceMismatch {
            expected: format!("{}{}", locked.amount, locked.denom),
            sent: format!("{}{}", total, locked.denom),
        });
    }

    let mut msgs = vec![];
    for split in splits {
        let addr = deps.api.addr_validate(&split.recipient)?;
        msgs.push(BankMsg::Send {
            to_address: addr.to_string(),
            amount: vec![Coin {
                denom: locked.denom.clone(),
                amount: split.amount,
            }],
        });
    }

    ask.status = AskStatus::Completed;
    ASKS.save(deps.storage, &ask_id, &ask)?;

    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "admin_resolve")
        .add_attribute("ask_id", ask_id))
}

fn admin_set_allowed_denoms(
    deps: DepsMut,
    info: MessageInfo,
    denoms: Vec<String>,
) -> Result<Response, ContractError> {
    nonpayable(&info)?;
    assert_owner(deps.storage, &info.sender)?;
    ALLOWED_DENOMS.save(deps.storage, &denoms)?;
    Ok(Response::new().add_attribute("action", "admin_set_allowed_denoms"))
}

// ---------------------------------------------------------------------------
// Query handlers
// ---------------------------------------------------------------------------

fn query_ask(deps: Deps, ask_id: String) -> StdResult<AskResponse> {
    let ask = ASKS.load(deps.storage, &ask_id)?;
    Ok(AskResponse { ask })
}

fn query_bid(deps: Deps, bid_id: String) -> StdResult<BidResponse> {
    let bid = bids().load(deps.storage, bid_id)?;
    Ok(BidResponse { bid })
}

fn query_bids_by_ask(
    deps: Deps,
    ask_id: String,
    start_after: Option<String>,
    limit: Option<u32>,
) -> StdResult<Vec<BidResponse>> {
    let limit = limit.unwrap_or(30).min(MAX_LIMIT) as usize;
    let start: Option<Bound<String>> = start_after.map(Bound::exclusive);

    bids()
        .idx
        .by_ask
        .prefix(ask_id)
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|r| {
            let (_, bid) = r?;
            Ok(BidResponse { bid })
        })
        .collect()
}

// Maximum number of results the caller may request per page
const MAX_LIMIT: u32 = 100;

fn query_asks(
    deps: Deps,
    start_after: Option<String>,
    limit: Option<u32>,
) -> StdResult<Vec<AskResponse>> {
    let limit = limit.unwrap_or(30).min(MAX_LIMIT) as usize;
    let start: Option<Bound<&str>> = start_after.as_deref().map(Bound::exclusive);

    ASKS.range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|r| Ok(AskResponse { ask: r?.1 }))
        .collect()
}

fn query_allowed_denoms(deps: Deps) -> StdResult<Vec<String>> {
    ALLOWED_DENOMS.load(deps.storage)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{MockApi, message_info, mock_dependencies, mock_env};
    use cosmwasm_std::{Addr, Uint128, coins, from_json};
    use cw_ownable::OwnershipError;

    const DENOM: &str = "uatom";

    /// Create a valid bech32 test address from a human-readable label.
    fn addr(label: &str) -> Addr {
        MockApi::default().addr_make(label)
    }

    fn setup(deps: DepsMut) {
        let owner = addr("owner");
        let msg = InstantiateMsg {
            owner: owner.to_string(),
            allowed_denoms: vec![],
        };
        instantiate(deps, mock_env(), message_info(&owner, &[]), msg).unwrap();
    }

    fn create_ask_and_get_id(deps: DepsMut) -> String {
        let client = addr("client");
        let res = execute(
            deps,
            mock_env(),
            message_info(&client, &[]),
            ExecuteMsg::CreateAsk {
                budget: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(100),
                },
                metadata_url: "https://example.com/ask-metadata.json".to_string(),
            },
        )
        .unwrap();
        res.attributes
            .iter()
            .find(|a| a.key == "ask_id")
            .unwrap()
            .value
            .clone()
    }

    fn submit_bid_and_get_id(
        deps: DepsMut,
        ask_id: &str,
        provider_label: &str,
        price: u128,
    ) -> String {
        let provider = addr(provider_label);
        let res = execute(
            deps,
            mock_env(),
            message_info(&provider, &[]),
            ExecuteMsg::SubmitBid {
                ask_id: ask_id.to_string(),
                price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(price),
                },
                funds_recipient: None,
            },
        )
        .unwrap();
        res.attributes
            .iter()
            .find(|a| a.key == "bid_id")
            .unwrap()
            .value
            .clone()
    }

    #[test]
    fn test_instantiate() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());
        let ownership = cw_ownable::get_ownership(&deps.storage).unwrap();
        assert_eq!(ownership.owner, Some(addr("owner")));
    }

    #[test]
    fn test_happy_path_full_flow() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let client = addr("client");
        let provider = addr("provider");

        // 1. Create ask
        let ask_id = create_ask_and_get_id(deps.as_mut());

        // 2. Submit two bids
        let bid_id = submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider", 80);
        let _bid_id2 = submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider2", 90);

        // 3. Select bid — client sends exact funds
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &coins(80, DENOM)),
            ExecuteMsg::SelectBid {
                ask_id: ask_id.clone(),
                bid_id: bid_id.clone(),
                expected_price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(80),
                },
            },
        )
        .unwrap();

        let ask = ASKS.load(&deps.storage, &ask_id).unwrap();
        assert!(matches!(ask.status, AskStatus::InProgress));
        assert_eq!(ask.locked_funds.unwrap().amount, Uint128::new(80));

        // 4. Provider marks complete
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&provider, &[]),
            ExecuteMsg::MarkComplete {
                ask_id: ask_id.clone(),
            },
        )
        .unwrap();

        let ask = ASKS.load(&deps.storage, &ask_id).unwrap();
        assert!(matches!(ask.status, AskStatus::WorkSubmitted));

        // 5. Client confirms — funds released to provider
        let res = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &[]),
            ExecuteMsg::ConfirmCompletion {
                ask_id: ask_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(res.messages.len(), 1);
        let ask = ASKS.load(&deps.storage, &ask_id).unwrap();
        assert!(matches!(ask.status, AskStatus::Completed));
    }

    #[test]
    fn test_cancel_open_ask() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let client = addr("client");
        let ask_id = create_ask_and_get_id(deps.as_mut());
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &[]),
            ExecuteMsg::CancelAsk {
                ask_id: ask_id.clone(),
            },
        )
        .unwrap();
        let ask = ASKS.load(&deps.storage, &ask_id).unwrap();
        assert!(matches!(ask.status, AskStatus::Cancelled));
    }

    #[test]
    fn test_cannot_cancel_inprogress_ask() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let client = addr("client");
        let ask_id = create_ask_and_get_id(deps.as_mut());
        let bid_id = submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider", 80);
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &coins(80, DENOM)),
            ExecuteMsg::SelectBid {
                ask_id: ask_id.clone(),
                bid_id,
                expected_price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(80),
                },
            },
        )
        .unwrap();

        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &[]),
            ExecuteMsg::CancelAsk {
                ask_id: ask_id.clone(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InvalidStatus { .. }));
    }

    #[test]
    fn test_duplicate_bid_rejected() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let provider = addr("provider");
        let ask_id = create_ask_and_get_id(deps.as_mut());
        submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider", 80);

        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&provider, &[]),
            ExecuteMsg::SubmitBid {
                ask_id: ask_id.clone(),
                price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(70),
                },
                funds_recipient: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::AlreadyBid {}));
    }

    #[test]
    fn test_select_bid_wrong_funds() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let client = addr("client");
        let ask_id = create_ask_and_get_id(deps.as_mut());
        let bid_id = submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider", 80);

        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &coins(50, DENOM)), // wrong amount
            ExecuteMsg::SelectBid {
                ask_id: ask_id.clone(),
                bid_id,
                expected_price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(80),
                },
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::BidPriceMismatch { .. }));
    }

    #[test]
    fn test_select_bid_wrong_ask() {
        // F-03: a bid from ask B cannot be selected for ask A
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let client = addr("client");
        let ask_a = create_ask_and_get_id(deps.as_mut());
        let ask_b = create_ask_and_get_id(deps.as_mut());
        // bid submitted for ask_b
        let bid_for_b = submit_bid_and_get_id(deps.as_mut(), &ask_b, "provider", 80);

        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &coins(80, DENOM)),
            ExecuteMsg::SelectBid {
                ask_id: ask_a.clone(), // trying to use bid from ask_b on ask_a
                bid_id: bid_for_b,
                expected_price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(80),
                },
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::BidNotFound { .. }));
    }

    #[test]
    fn test_select_bid_front_run_protection() {
        // Selecting a bid with an expected_price that differs from the stored price must fail
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let client = addr("client");
        let ask_id = create_ask_and_get_id(deps.as_mut());
        let bid_id = submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider", 80);

        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &coins(80, DENOM)),
            ExecuteMsg::SelectBid {
                ask_id: ask_id.clone(),
                bid_id,
                expected_price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(90), // stale/wrong expected price
                },
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::BidPriceMismatch { .. }));
    }

    #[test]
    fn test_submit_bid_zero_price() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let ask_id = create_ask_and_get_id(deps.as_mut());
        let provider = addr("provider");

        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&provider, &[]),
            ExecuteMsg::SubmitBid {
                ask_id,
                price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::zero(),
                },
                funds_recipient: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InsufficientFunds {}));
    }

    #[test]
    fn test_submit_bid_wrong_denom() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let ask_id = create_ask_and_get_id(deps.as_mut());
        let provider = addr("provider");

        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&provider, &[]),
            ExecuteMsg::SubmitBid {
                ask_id,
                price: Coin {
                    denom: "uusdc".to_string(), // ask budget is uatom
                    amount: Uint128::new(80),
                },
                funds_recipient: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::BidPriceMismatch { .. }));
    }

    #[test]
    fn test_partial_disbursement() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let client = addr("client");
        let ask_id = create_ask_and_get_id(deps.as_mut());
        let bid_id = submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider", 100);
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &coins(100, DENOM)),
            ExecuteMsg::SelectBid {
                ask_id: ask_id.clone(),
                bid_id,
                expected_price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(100),
                },
            },
        )
        .unwrap();

        // Client releases 30 upfront as advance
        let res = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &[]),
            ExecuteMsg::PartialDisbursement {
                ask_id: ask_id.clone(),
                amount: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(30),
                },
            },
        )
        .unwrap();
        assert_eq!(res.messages.len(), 1);

        let ask = ASKS.load(&deps.storage, &ask_id).unwrap();
        assert_eq!(ask.locked_funds.unwrap().amount, Uint128::new(70));
    }

    #[test]
    fn test_owner_partial_disbursement_rejected() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let owner = addr("owner");
        let ask_id = create_ask_and_get_id(deps.as_mut());
        let bid_id = submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider", 100);
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&addr("client"), &coins(100, DENOM)),
            ExecuteMsg::SelectBid {
                ask_id: ask_id.clone(),
                bid_id,
                expected_price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(100),
                },
            },
        )
        .unwrap();

        // Owner is no longer allowed to call partial_disbursement; use admin_force_release instead
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            ExecuteMsg::PartialDisbursement {
                ask_id: ask_id.clone(),
                amount: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(50),
                },
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::NotAskCreator {}));
    }

    #[test]
    fn test_admin_cancel_with_refund() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let owner = addr("owner");
        let client = addr("client");
        let ask_id = create_ask_and_get_id(deps.as_mut());
        let bid_id = submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider", 80);
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &coins(80, DENOM)),
            ExecuteMsg::SelectBid {
                ask_id: ask_id.clone(),
                bid_id,
                expected_price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(80),
                },
            },
        )
        .unwrap();

        let res = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            ExecuteMsg::AdminCancel {
                ask_id: ask_id.clone(),
                refund_to: client.to_string(),
            },
        )
        .unwrap();

        assert_eq!(res.messages.len(), 1); // refund BankMsg
        let ask = ASKS.load(&deps.storage, &ask_id).unwrap();
        assert!(matches!(ask.status, AskStatus::Cancelled));
    }

    #[test]
    fn test_admin_force_release() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let owner = addr("owner");
        let client = addr("client");
        let provider = addr("provider");
        let ask_id = create_ask_and_get_id(deps.as_mut());
        let bid_id = submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider", 80);
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &coins(80, DENOM)),
            ExecuteMsg::SelectBid {
                ask_id: ask_id.clone(),
                bid_id,
                expected_price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(80),
                },
            },
        )
        .unwrap();

        let res = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            ExecuteMsg::AdminForceRelease {
                ask_id: ask_id.clone(),
                recipient: provider.to_string(),
            },
        )
        .unwrap();

        assert_eq!(res.messages.len(), 1);
        let ask = ASKS.load(&deps.storage, &ask_id).unwrap();
        assert!(matches!(ask.status, AskStatus::Completed));
    }

    #[test]
    fn test_admin_resolve_split() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let owner = addr("owner");
        let client = addr("client");
        let provider = addr("provider");
        let ask_id = create_ask_and_get_id(deps.as_mut());
        let bid_id = submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider", 100);
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &coins(100, DENOM)),
            ExecuteMsg::SelectBid {
                ask_id: ask_id.clone(),
                bid_id,
                expected_price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(100),
                },
            },
        )
        .unwrap();

        // Split 60 to provider, 40 back to client
        let res = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            ExecuteMsg::AdminResolve {
                ask_id: ask_id.clone(),
                splits: vec![
                    RecipientShare {
                        recipient: provider.to_string(),
                        amount: Uint128::new(60),
                    },
                    RecipientShare {
                        recipient: client.to_string(),
                        amount: Uint128::new(40),
                    },
                ],
            },
        )
        .unwrap();

        assert_eq!(res.messages.len(), 2);
        let ask = ASKS.load(&deps.storage, &ask_id).unwrap();
        assert!(matches!(ask.status, AskStatus::Completed));
        assert!(ask.locked_funds.is_none());
    }

    #[test]
    fn test_admin_resolve_rejects_wrong_sum() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let owner = addr("owner");
        let client = addr("client");
        let provider = addr("provider");
        let ask_id = create_ask_and_get_id(deps.as_mut());
        let bid_id = submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider", 100);
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &coins(100, DENOM)),
            ExecuteMsg::SelectBid {
                ask_id: ask_id.clone(),
                bid_id,
                expected_price: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(100),
                },
            },
        )
        .unwrap();

        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            ExecuteMsg::AdminResolve {
                ask_id: ask_id.clone(),
                splits: vec![RecipientShare {
                    recipient: provider.to_string(),
                    amount: Uint128::new(60), // only 60 of 100 — does not sum to locked amount
                }],
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::BidPriceMismatch { .. }));
    }

    #[test]
    fn test_admin_resolve_rejects_too_many_recipients() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let owner = addr("owner");
        let ask_id = create_ask_and_get_id(deps.as_mut());

        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            ExecuteMsg::AdminResolve {
                ask_id,
                splits: (0..6)
                    .map(|i| RecipientShare {
                        recipient: addr(&format!("r{i}")).to_string(),
                        amount: Uint128::new(10),
                    })
                    .collect(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InvalidRecipientsCount { .. }));
    }

    #[test]
    fn test_query_list_bids_by_ask() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let ask_id = create_ask_and_get_id(deps.as_mut());
        submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider", 80);
        submit_bid_and_get_id(deps.as_mut(), &ask_id, "provider2", 90);

        let res: Vec<BidResponse> = from_json(
            query(
                deps.as_ref(),
                mock_env(),
                QueryMsg::ListBidsByAsk {
                    ask_id: ask_id.clone(),
                    start_after: None,
                    limit: None,
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(res.len(), 2);
    }

    #[test]
    fn test_query_list_asks_by_status() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        create_ask_and_get_id(deps.as_mut());
        create_ask_and_get_id(deps.as_mut());

        let res: Vec<AskResponse> = from_json(
            query(
                deps.as_ref(),
                mock_env(),
                QueryMsg::ListAsks {
                    start_after: None,
                    limit: None,
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(res.len(), 2);
    }

    #[test]
    fn test_query_ownership() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let res: cw_ownable::Ownership<Addr> =
            from_json(query(deps.as_ref(), mock_env(), QueryMsg::Ownership {}).unwrap()).unwrap();

        assert_eq!(res.owner, Some(addr("owner")));
    }

    #[test]
    fn test_unauthorized_admin_action() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let client = addr("client");
        let ask_id = create_ask_and_get_id(deps.as_mut());
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&client, &[]), // not owner
            ExecuteMsg::AdminCancel {
                ask_id,
                refund_to: client.to_string(),
            },
        )
        .unwrap_err();
        // cw_ownable returns OwnershipError::NotOwner
        assert!(matches!(
            err,
            ContractError::Ownership(OwnershipError::NotOwner)
        ));
    }

    #[test]
    fn test_ownership_transfer() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let owner = addr("owner");
        let new_owner = addr("new_owner");

        // Initiate 2-step transfer
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            ExecuteMsg::UpdateOwnership(cw_ownable::Action::TransferOwnership {
                new_owner: new_owner.to_string(),
                expiry: None,
            }),
        )
        .unwrap();

        // New owner accepts
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&new_owner, &[]),
            ExecuteMsg::UpdateOwnership(cw_ownable::Action::AcceptOwnership {}),
        )
        .unwrap();

        let ownership = cw_ownable::get_ownership(&deps.storage).unwrap();
        assert_eq!(ownership.owner, Some(new_owner));
    }

    // ---------------------------------------------------------------------------
    // validate_url tests
    // ---------------------------------------------------------------------------

    fn try_create_ask_url(deps: DepsMut, url: &str) -> Result<Response, ContractError> {
        execute(
            deps,
            mock_env(),
            message_info(&addr("client"), &[]),
            ExecuteMsg::CreateAsk {
                budget: Coin {
                    denom: DENOM.to_string(),
                    amount: Uint128::new(100),
                },
                metadata_url: url.to_string(),
            },
        )
    }

    #[test]
    fn test_metadata_url_valid_schemes() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        // All three supported schemes must pass
        assert!(try_create_ask_url(deps.as_mut(), "https://example.com/meta.json").is_ok());
        assert!(try_create_ask_url(deps.as_mut(), "ipfs://bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi").is_ok());
        assert!(try_create_ask_url(deps.as_mut(), "ar://abc123").is_ok());
    }

    #[test]
    fn test_metadata_url_empty_rejected() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let err = try_create_ask_url(deps.as_mut(), "").unwrap_err();
        assert!(matches!(err, ContractError::InvalidMetadataUrl {}));
    }

    #[test]
    fn test_metadata_url_too_long_rejected() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        let long_url = format!("https://example.com/{}", "a".repeat(MAX_METADATA_URL_LEN));
        let err = try_create_ask_url(deps.as_mut(), &long_url).unwrap_err();
        assert!(matches!(err, ContractError::InvalidMetadataUrl {}));
    }

    #[test]
    fn test_metadata_url_unsupported_scheme_rejected() {
        let mut deps = mock_dependencies();
        setup(deps.as_mut());

        for bad_url in &[
            "http://example.com/meta.json",   // http not allowed
            "ftp://example.com/meta.json",    // ftp not allowed
            "example.com/meta.json",          // no scheme
            "//example.com/meta.json",        // scheme-relative
        ] {
            let err = try_create_ask_url(deps.as_mut(), bad_url).unwrap_err();
            assert!(
                matches!(err, ContractError::InvalidMetadataUrl {}),
                "expected rejection for: {bad_url}"
            );
        }
    }
}
