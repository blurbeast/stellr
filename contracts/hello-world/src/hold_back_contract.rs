use crate::entities::*;
use crate::errors::*;
use soroban_sdk::{contract, contractimpl, log, token, Symbol, Address, Env};

pub const DAY_IN_SECONDS: u64 = 86400;

#[contract]
pub struct HoldBackContract;

#[contractimpl]
impl HoldBackContract {
    pub fn initialize(env: &Env, admin: Address) -> Result<bool, Error> {
        admin.require_auth();
        if env.storage().persistent().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        Ok(true)
    }

    pub fn create_payment(
        env: Env,
        buyer: Address,
        seller: Address,
        amount: u128,
        token: Address,
        holdback_rate: u32,
        holdback_days: u32,
    ) -> Result<u128, Error> {
        buyer.require_auth();
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;

        if amount == 0 || amount as i128 > i128::MAX {
            return Err(Error::InvalidAmount);
        }
        if holdback_rate == 0 || holdback_rate > 100 {
            return Err(Error::InvalidHoldbackRate);
        }
        if buyer == seller || buyer == admin || seller == admin {
            return Err(Error::InvalidBuyer);
        }
        if buyer == token || seller == token {
            return Err(Error::InvalidSeller);
        }

        let holdback_amount = (amount * holdback_rate as u128) / 100;
        let final_amount = amount
            .checked_sub(holdback_amount)
            .ok_or(Error::InvalidAmount)?;

        let token_client = token::Client::new(&env, &token);
        let bal = token_client.balance(&buyer);
        if bal < amount as i128 {
            return Err(Error::InsufficientBalance);
        }
        let allowance = token_client.allowance(&buyer, &env.current_contract_address());
        if allowance < amount as i128 {
            return Err(Error::InsufficientAllowance);
        }
        token_client
            .transfer_from(&env.current_contract_address(), &buyer, &env.current_contract_address(), &(amount as i128));

        if final_amount > 0 {
            token_client.transfer(
                &env.current_contract_address(),
                &seller,
                &(final_amount as i128),
            );
        }

        let transaction_id = env
            .storage()
            .persistent()
            .get(&DataKey::TransactionCounter)
            .unwrap_or(0u128)
            .checked_add(1)
            .ok_or(Error::InvalidAmount)?;
        
        env.storage()
            .persistent()
            .set(&DataKey::TransactionCounter, &transaction_id);
        
        let release_time = env.ledger()
            .timestamp()
            .checked_add((holdback_days as u64).saturating_mul(DAY_IN_SECONDS))
            .ok_or(Error::InvalidAmount)?;


        let transaction = Transaction {
            buyer: buyer.clone(),
            seller: seller.clone(),
            amount,
            token,
            holdback_rate,
            holdback_amount,
            final_amount,
            release_time,
            status: TransactionStatus::Held,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Transaction(transaction_id), &transaction);

        // env.events().publish(
        //     ("transaction_created",),
        //     (transaction_id, buyer, seller, amount, holdback_amount),
        // );
        // 
        env.events().publish(
            (Symbol::short("tx_created"),),
            (transaction_id, buyer, seller, amount, holdback_amount),
        );

        log!(
            &env,
            "Transaction {} created with holdback {}%",
            transaction_id,
            holdback_rate
        );
        Ok(transaction_id)
    }

    pub fn approve_release(env: &Env, transaction_id: u128, buyer: Address) -> Result<(), Error> {
        buyer.require_auth();
        let mut transaction: Transaction = env
            .storage()
            .persistent()
            .get(&DataKey::Transaction(transaction_id))
            .ok_or(Error::TransactionNotFound)?;
        if transaction.buyer != buyer {
            return Err(Error::Unauthorized);
        }
        if transaction.status != TransactionStatus::Held {
            return Err(Error::InvalidStatus);
        }

        transaction.status = TransactionStatus::HoldbackPending;
        env.storage()
            .persistent()
            .set(&DataKey::Transaction(transaction_id), &transaction);

        Self::release_holdback_if_due(&env, transaction_id)?;
        Ok(())
    }

    pub fn initiate_dispute(env: &Env, transaction_id: u128, buyer: Address) -> Result<(), Error> {
        buyer.require_auth();
        let mut transaction: Transaction = env
            .storage()
            .persistent()
            .get(&DataKey::Transaction(transaction_id))
            .ok_or(Error::TransactionNotFound)?;
        if transaction.buyer != buyer {
            return Err(Error::Unauthorized);
        }
        if transaction.status != TransactionStatus::Held
            && transaction.status != TransactionStatus::HoldbackPending
        {
            return Err(Error::InvalidStatus);
        }

        transaction.status = TransactionStatus::Disputed;
        env.storage()
            .persistent()
            .set(&DataKey::Transaction(transaction_id), &transaction);

        env.events()
            .publish(("dispute_initiated",), (transaction_id, buyer));
        Ok(())
    }

    pub fn resolve_dispute(
        env: &Env,
        transaction_id: u128,
        refund: bool,
        admin: Address,
    ) -> Result<(), Error> {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        if admin != stored_admin {
            return Err(Error::Unauthorized);
        }

        let mut transaction: Transaction = env
            .storage()
            .persistent()
            .get(&DataKey::Transaction(transaction_id))
            .ok_or(Error::TransactionNotFound)?;
        if transaction.status != TransactionStatus::Disputed {
            return Err(Error::InvalidStatus);
        }

        let token_client = token::Client::new(&env, &transaction.token);
        if refund {
            token_client.transfer(
                &env.current_contract_address(),
                &transaction.buyer,
                &(transaction.holdback_amount as i128),
            );
            transaction.status = TransactionStatus::Cancelled;
            env.events().publish(
                ("holdback_refunded",),
                (
                    transaction_id,
                    transaction.buyer.clone(),
                    transaction.holdback_amount,
                ),
            );
        } else {
            token_client.transfer(
                &env.current_contract_address(),
                &transaction.seller,
                &(transaction.holdback_amount as i128),
            );
            transaction.status = TransactionStatus::Completed;
            env.events().publish(
                ("holdback_released",),
                (
                    transaction_id,
                    transaction.seller.clone(),
                    transaction.holdback_amount,
                ),
            );
        }
        env.storage()
            .persistent()
            .set(&DataKey::Transaction(transaction_id), &transaction);
        Ok(())
    }

    pub fn check_and_release(env: &Env, transaction_id: u128) -> Result<(), Error> {
        let transaction: Transaction = env
            .storage()
            .persistent()
            .get(&DataKey::Transaction(transaction_id))
            .ok_or(Error::TransactionNotFound)?;
        // if transaction.status != TransactionStatus::Held
        // || transaction.status != TransactionStatus::HoldbackPending
        if matches!(transaction.status, TransactionStatus::Disputed) {
            return Err(Error::InvalidStatus);
        }

        Self::release_holdback_if_due(&env, transaction_id)?;
        Ok(())
    }

    fn release_holdback_if_due(env: &Env, transaction_id: u128) -> Result<(), Error> {
        let mut transaction: Transaction = env
            .storage()
            .persistent()
            .get(&DataKey::Transaction(transaction_id))
            .ok_or(Error::TransactionNotFound)?;

        if transaction.status == TransactionStatus::HoldbackPending
            || (transaction.status == TransactionStatus::Held
                && env.ledger().timestamp() >= transaction.release_time)
        {
            let token_client = token::Client::new(&env, &transaction.token);
            token_client.transfer(
                &env.current_contract_address(),
                &transaction.seller,
                &(transaction.holdback_amount as i128),
            );
            transaction.status = TransactionStatus::Completed;
            env.storage()
                .persistent()
                .set(&DataKey::Transaction(transaction_id), &transaction);

            env.events().publish(
                ("holdback_released",),
                (
                    transaction_id,
                    transaction.seller,
                    transaction.holdback_amount,
                ),
            );
        }
        Ok(())
    }

    pub fn get_transaction(env: &Env, transaction_id: u128) -> Result<Transaction, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Transaction(transaction_id))
            .ok_or(Error::TransactionNotFound)
    }

    pub fn get_admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }
}
