use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Coin};
use cw_storage_plus::{IndexedMap, Item, Map, MultiIndex, UniqueIndex, index_list};

#[cw_serde]
pub enum AskStatus {
    Open,
    InProgress,
    WorkSubmitted,
    Completed,
    Cancelled,
}

#[cw_serde]
pub struct Ask {
    pub id: String,
    pub creator: Addr,
    pub budget: Coin,
    pub metadata_url: String,
    pub status: AskStatus,
    pub selected_bid: Option<String>,
    pub locked_funds: Option<Coin>, // set when SelectBid is executed
}

#[cw_serde]
pub struct Bid {
    pub id: String,
    pub ask_id: String,
    pub provider: Addr,
    pub price: Coin,
    // defaults to provider if None
    pub funds_recipient: Option<Addr>,
}

impl Bid {
    /// Returns the address that should receive funds: `funds_recipient` if set,
    /// otherwise falls back to `provider`.
    pub fn payout_address(&self) -> &Addr {
        self.funds_recipient.as_ref().unwrap_or(&self.provider)
    }
}

#[index_list(Bid)]
pub struct BidIndices<'a> {
    /// All bids for a given ask (ask_id → bid)
    pub by_ask: MultiIndex<'a, String, Bid, String>,
    /// Prevents a provider from bidding twice on the same ask ((ask_id, provider) → bid)
    pub by_ask_provider: UniqueIndex<'a, (String, String), Bid, String>,
}

pub fn bids<'a>() -> IndexedMap<String, Bid, BidIndices<'a>> {
    IndexedMap::new(
        "bids",
        BidIndices {
            by_ask: MultiIndex::new(|_pk, bid: &Bid| bid.ask_id.clone(), "bids", "bids_ask"),
            by_ask_provider: UniqueIndex::new(
                |bid: &Bid| (bid.ask_id.clone(), bid.provider.to_string()),
                "bids_prov",
            ),
        },
    )
}

pub const ASK_COUNT: Item<u64> = Item::new("ask_count"); // nonce for ID hash
pub const BID_COUNT: Item<u64> = Item::new("bid_count");

pub const ASKS: Map<&str, Ask> = Map::new("asks");

/// Allowlisted payment denoms. Empty = all denoms accepted.
pub const ALLOWED_DENOMS: Item<Vec<String>> = Item::new("allowed_denoms");
