use blake2::{Blake2s256, Digest};
use cosmwasm_std::{Addr, Api, Env, MessageInfo, Storage};
use cw_ownable::assert_owner;

use crate::ContractError;
use crate::state::{ASK_COUNT, Ask, BID_COUNT};

/// Validate an `allowed_denoms` list: must be non-empty, contain no empty
/// strings, and contain no duplicates. Returning an error here is preferable
/// to silently normalizing because the caller's intent is ambiguous if, say,
/// they passed two copies of the same denom.
pub fn validate_allowed_denoms(denoms: &[String]) -> Result<(), ContractError> {
    if denoms.is_empty() {
        return Err(ContractError::InvalidAllowedDenoms {});
    }
    let mut seen = std::collections::BTreeSet::new();
    for d in denoms {
        if d.is_empty() || !seen.insert(d.as_str()) {
            return Err(ContractError::InvalidAllowedDenoms {});
        }
    }
    Ok(())
}

pub fn validate_payout_address(api: &dyn Api, env: &Env, raw: &str) -> Result<Addr, ContractError> {
    let addr = api.addr_validate(raw)?;
    if addr == env.contract.address {
        return Err(ContractError::InvalidPayoutAddress {});
    }
    Ok(addr)
}

/// Domain separator between pre-image fields. `|` (0x7C) is outside the
/// bech32 charset and the decimal digit set, so no input component can
/// produce a byte sequence that collides with the separator.
const ID_FIELD_SEPARATOR: &[u8] = b"|";

pub fn generate_id(parts: Vec<&[u8]>) -> String {
    let mut hasher = Blake2s256::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hasher.update(ID_FIELD_SEPARATOR);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_id_disambiguates_adjacent_fields() {
        // Without a separator the two tuples below produce the same
        // concatenated pre-image ("ask" + "12" + "345" == "ask" + "123" + "45").
        let a = generate_id(vec![b"ask", b"12", b"345"]);
        let b = generate_id(vec![b"ask", b"123", b"45"]);
        assert_ne!(a, b);
    }
}
