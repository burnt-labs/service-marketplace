use cosmwasm_std::StdError;
use cw_ownable::OwnershipError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error(transparent)]
    Ownership(#[from] OwnershipError),

    #[error(transparent)]
    Payment(#[from] cw_utils::PaymentError),

    #[error("Ask not found: {id}")]
    AskNotFound { id: String },

    #[error("Bid not found: {id}")]
    BidNotFound { id: String },

    #[error("Invalid status: expected {expected}, got {got}")]
    InvalidStatus { expected: String, got: String },

    #[error("Insufficient funds")]
    InsufficientFunds {},

    #[error("Bid price mismatch: expected {expected}, sent {sent}")]
    BidPriceMismatch { expected: String, sent: String },

    #[error("Already bid on this ask")]
    AlreadyBid {},

    #[error("Not the ask creator")]
    NotAskCreator {},

    #[error("Not the bid provider")]
    NotBidProvider {},

    #[error("Not owner or ask creator")]
    NotOwnerOrCreator {},

    #[error("No selected bid on this ask")]
    NoSelectedBid {},

    #[error("AdminResolve requires between {min} and {max} recipients")]
    InvalidRecipientsCount { min: usize, max: usize },

    #[error("Denom '{denom}' is not allowed")]
    DenomNotAllowed { denom: String },

    #[error("Ask creator cannot bid on their own ask")]
    CreatorCannotBid {},
}
