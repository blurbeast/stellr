
use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum Error {
    NotInitialized = 1,
    InvalidAmount = 2,
    InvalidHoldbackRate = 3,
    InvalidBuyer = 4,
    InvalidSeller = 5,
    TransactionNotFound = 6,
    InvalidStatus = 7,
    Unauthorized = 8,
    AlreadyInitialized = 9,
    Paused = 10,
    RateLimitExceeded = 11,
}
