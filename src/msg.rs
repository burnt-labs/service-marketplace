use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Coin, Uint128};
use cw_ownable::{cw_ownable_execute, cw_ownable_query};

use crate::state::{Ask, Bid};

#[cw_serde]
pub struct InstantiateMsg {
    /// Initial owner (admin). Supports 2-step transfers via UpdateOwnership.
    pub owner: String,
    /// Denoms accepted for ask budgets and bid prices. Empty = all denoms allowed.
    pub allowed_denoms: Vec<String>,
}

#[cw_ownable_execute]
#[cw_serde]
pub enum ExecuteMsg {
    CreateAsk {
        budget: Coin,
    },
    SelectBid {
        ask_id: String,
        bid_id: String,
        expected_price: Coin,
    },
    PartialDisbursement {
        ask_id: String,
        amount: Coin,
    },
    ConfirmCompletion {
        ask_id: String,
    },
    CancelAsk {
        ask_id: String,
    },
    SubmitBid {
        ask_id: String,
        price: Coin,
        /// Override payout address. Defaults to sender if omitted.
        funds_recipient: Option<String>,
    },
    /// Mark work as complete. Transitions InProgress → WorkSubmitted.
    MarkComplete {
        ask_id: String,
    },

    // --- Owner only ---
    /// Cancel any active ask and refund locked funds.
    AdminCancel {
        ask_id: String,
        refund_to: String,
    },
    /// Force-release all locked funds to a given recipient.
    AdminForceRelease {
        ask_id: String,
        recipient: String,
    },
    /// Resolve a dispute by splitting locked funds across up to MAX_RESOLVE_RECIPIENTS.
    /// All splits must share the same denom as the locked funds and sum to exactly the locked amount.
    AdminResolve {
        ask_id: String,
        splits: Vec<RecipientShare>,
    },
    /// Replace the allowlisted denoms. Pass an empty list to allow all denoms.
    AdminSetAllowedDenoms {
        denoms: Vec<String>,
    },
}

#[cw_ownable_query]
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(AskResponse)]
    GetAsk { ask_id: String },

    #[returns(BidResponse)]
    GetBid { bid_id: String },

    #[returns(Vec<BidResponse>)]
    ListBidsByAsk {
        ask_id: String,
        start_after: Option<String>,
        limit: Option<u32>,
    },

    #[returns(Vec<AskResponse>)]
    ListAsks {
        start_after: Option<String>,
        limit: Option<u32>,
    },

    #[returns(Vec<String>)]
    GetAllowedDenoms {},
}

/// One entry in an AdminResolve split: a recipient address and the amount
/// (in the ask's locked-funds denom) they should receive.
#[cw_serde]
pub struct RecipientShare {
    pub recipient: String,
    pub amount: Uint128,
}

#[cw_serde]
pub struct AskResponse {
    pub ask: Ask,
}

#[cw_serde]
pub struct BidResponse {
    pub bid: Bid,
}
