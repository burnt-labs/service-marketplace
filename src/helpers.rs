use blake2::{Blake2s256, Digest};
use cosmwasm_std::{Addr, Api, Env, MessageInfo, Storage};
use cw_ownable::assert_owner;

use crate::ContractError;
use crate::state::{ASK_COUNT, Ask, BID_COUNT};

pub fn validate_payout_address(
    api: &dyn Api,
    env: &Env,
    raw: &str,
) -> Result<Addr, ContractError> {
    let addr = api.addr_validate(raw)?;
    if addr == env.contract.address {
        return Err(ContractError::InvalidPayoutAddress {});
    }
    Ok(addr)
}

pub fn generate_id(parts: Vec<&[u8]>) -> String {
    let mut hasher = Blake2s256::new();
    for part in parts {
        hasher.update(part);
    }
    format!("{:x}", hasher.finalize())
}

/// Increment ASK_COUNT (wrapping) and return a new unique ask ID.
pub fn next_ask_id(
    storage: &mut dyn Storage,
    env: &Env,
    info: &MessageInfo,
) -> Result<String, ContractError> {
    let nonce = ASK_COUNT.load(storage).unwrap_or(0).wrapping_add(1);
    ASK_COUNT.save(storage, &nonce)?;
    let height = env.block.height.to_string();
    let time = env.block.time.nanos().to_string();
    let tx_index = env
        .transaction
        .as_ref()
        .map(|t| t.index)
        .unwrap_or(0)
        .to_string();
    let nonce_str = nonce.to_string();
    let hash = generate_id(vec![
        b"ask",
        height.as_bytes(),
        time.as_bytes(),
        tx_index.as_bytes(),
        info.sender.as_bytes(),
        nonce_str.as_bytes(),
    ]);
    Ok(hash)
}

/// Increment BID_COUNT (wrapping) and return a new unique bid ID.
pub fn next_bid_id(
    storage: &mut dyn Storage,
    env: &Env,
    info: &MessageInfo,
) -> Result<String, ContractError> {
    let nonce = BID_COUNT.load(storage).unwrap_or(0).wrapping_add(1);
    BID_COUNT.save(storage, &nonce)?;
    let height = env.block.height.to_string();
    let time = env.block.time.nanos().to_string();
    let tx_index = env
        .transaction
        .as_ref()
        .map(|t| t.index)
        .unwrap_or(0)
        .to_string();
    let nonce_str = nonce.to_string();
    let hash = generate_id(vec![
        b"bid",
        height.as_bytes(),
        time.as_bytes(),
        tx_index.as_bytes(),
        info.sender.as_bytes(),
        nonce_str.as_bytes(),
    ]);
    Ok(hash)
}

/// Assert that the message sender is the ask creator.
pub fn assert_creator(ask: &Ask, info: &MessageInfo) -> Result<(), ContractError> {
    if ask.creator != info.sender {
        return Err(ContractError::NotAskCreator {});
    }
    Ok(())
}

/// Assert that the message sender is either the contract owner or the ask creator.
/// Checks creator first (cheaper), falls back to ownership check.
pub fn assert_owner_or_creator(
    storage: &dyn Storage,
    ask: &Ask,
    info: &MessageInfo,
) -> Result<(), ContractError> {
    if ask.creator == info.sender {
        return Ok(());
    }
    assert_owner(storage, &info.sender).map_err(|_| ContractError::NotOwnerOrCreator {})
}
