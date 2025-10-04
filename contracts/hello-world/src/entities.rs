use soroban_sdk::{contracttype, Address};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionStatus {
    Held,
    HoldbackPending,
    Completed,
    Cancelled,
    Disputed,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transaction {
    pub buyer: Address,
    pub seller: Address,
    pub amount: u128,
    pub token: Address,
    pub holdback_rate: u32,
    pub holdback_amount: u128,
    pub final_amount: u128,
    pub release_time: u64,
    pub status: TransactionStatus,
}

#[contracttype]
#[derive(Debug, Eq, PartialEq)]
pub enum DataKey {
    Transaction(u128),
    TransactionCounter,
    Token,
    Admin,
}
