#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env,
    Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    ThresholdNotMet = 4,
    TimelockActive = 5,
    ProposalNotFound = 6,
    ProposalAlreadyExecuted = 7,
    ProposalVetoed = 8,
    StreamNotFound = 9,
    StreamExpired = 10,
    ZeroAmount = 11,
    InvalidTimelock = 12,
    SignerAlreadyApproved = 13,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryConfig {
    pub signers: Vec<Address>,
    pub threshold: u32,
    pub timelock_delay_seconds: u64,
    pub guardian: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    pub recipient: Address,
    pub amount: i128,
    pub eta: u64,
    pub approvals: Vec<Address>,
    pub executed: bool,
    pub vetoed: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamSchedule {
    pub id: u64,
    pub sender: Address,
    pub recipient: Address,
    pub total_amount: i128,
    pub claimed_amount: i128,
    pub start_time: u64,
    pub stop_time: u64,
    pub is_cancelled: bool,
}

#[contracttype]
pub enum DataKey {
    Config,
    ProposalCounter,
    Proposal(u64),
    StreamCounter,
    Stream(u64),
}

const INSTANCE_BUMP_AMOUNT: u32 = 50_000;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 10_000;
const PERSISTENT_BUMP_AMOUNT: u32 = 100_000;
const PERSISTENT_LIFETIME_THRESHOLD: u32 = 20_000;

#[contract]
pub struct MultisigTimelockContract;

#[contractimpl]
impl MultisigTimelockContract {
    /// Initializes the multisig treasury with threshold signers, delay, and emergency guardian.
    pub fn initialize(
        env: Env,
        signers: Vec<Address>,
        threshold: u32,
        timelock_delay_seconds: u64,
        guardian: Address,
    ) -> Result<(), ContractError> {
        if env.storage().instance().has(&DataKey::Config) {
            return Err(ContractError::AlreadyInitialized);
        }

        if threshold == 0 || threshold > signers.len() {
            return Err(ContractError::ThresholdNotMet);
        }

        let config = TreasuryConfig {
            signers,
            threshold,
            timelock_delay_seconds,
            guardian,
        };

        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().set(&DataKey::ProposalCounter, &0u64);
        env.storage().instance().set(&DataKey::StreamCounter, &0u64);

        Self::bump_instance_ttl(&env);
        Ok(())
    }

    /// Proposes a treasury disbursement. Must be called by an authorized signer.
    pub fn propose_transaction(
        env: Env,
        proposer: Address,
        recipient: Address,
        amount: i128,
    ) -> Result<u64, ContractError> {
        proposer.require_auth();
        Self::bump_instance_ttl(&env);

        let config: TreasuryConfig = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .ok_or(ContractError::NotInitialized)?;

        if !config.signers.contains(&proposer) {
            return Err(ContractError::Unauthorized);
        }
        if amount <= 0 {
            return Err(ContractError::ZeroAmount);
        }

        let mut counter: u64 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalCounter)
            .unwrap_or(0);
        counter += 1;
        env.storage().instance().set(&DataKey::ProposalCounter, &counter);

        let current_time = env.ledger().timestamp();
        let eta = current_time + config.timelock_delay_seconds;

        let mut approvals = Vec::new(&env);
        approvals.push_back(proposer.clone());

        let proposal = Proposal {
            id: counter,
            proposer,
            recipient,
            amount,
            eta,
            approvals,
            executed: false,
            vetoed: false,
        };

        let key = DataKey::Proposal(counter);
        env.storage().persistent().set(&key, &proposal);
        Self::bump_persistent_ttl(&env, &key);

        Ok(counter)
    }

    /// Approves a proposed transaction.
    pub fn approve_proposal(
        env: Env,
        signer: Address,
        proposal_id: u64,
    ) -> Result<(), ContractError> {
        signer.require_auth();
        Self::bump_instance_ttl(&env);

        let config: TreasuryConfig = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .ok_or(ContractError::NotInitialized)?;

        if !config.signers.contains(&signer) {
            return Err(ContractError::Unauthorized);
        }

        let key = DataKey::Proposal(proposal_id);
        let mut proposal: Proposal = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(ContractError::ProposalNotFound)?;

        if proposal.executed {
            return Err(ContractError::ProposalAlreadyExecuted);
        }
        if proposal.vetoed {
            return Err(ContractError::ProposalVetoed);
        }
        if proposal.approvals.contains(&signer) {
            return Err(ContractError::SignerAlreadyApproved);
        }

        proposal.approvals.push_back(signer);
        env.storage().persistent().set(&key, &proposal);
        Self::bump_persistent_ttl(&env, &key);

        Ok(())
    }

    /// Executes an approved proposal after threshold reached and timelock elapsed.
    pub fn execute_proposal(env: Env, proposal_id: u64) -> Result<(), ContractError> {
        Self::bump_instance_ttl(&env);

        let config: TreasuryConfig = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .ok_or(ContractError::NotInitialized)?;

        let key = DataKey::Proposal(proposal_id);
        let mut proposal: Proposal = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(ContractError::ProposalNotFound)?;

        if proposal.executed {
            return Err(ContractError::ProposalAlreadyExecuted);
        }
        if proposal.vetoed {
            return Err(ContractError::ProposalVetoed);
        }
        if proposal.approvals.len() < config.threshold {
            return Err(ContractError::ThresholdNotMet);
        }

        let current_time = env.ledger().timestamp();
        if current_time < proposal.eta {
            return Err(ContractError::TimelockActive);
        }

        proposal.executed = true;
        env.storage().persistent().set(&key, &proposal);
        Self::bump_persistent_ttl(&env, &key);

        // State write completed before external event/transfer
        env.events().publish(
            (symbol_short!("multisig"), symbol_short!("executed")),
            proposal.id,
        );

        Ok(())
    }

    /// Emergency Veto: Guardian or Security Council can instantly cancel a proposal.
    pub fn emergency_veto(env: Env, caller: Address, proposal_id: u64) -> Result<(), ContractError> {
        caller.require_auth();
        Self::bump_instance_ttl(&env);

        let config: TreasuryConfig = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .ok_or(ContractError::NotInitialized)?;

        if caller != config.guardian {
            return Err(ContractError::Unauthorized);
        }

        let key = DataKey::Proposal(proposal_id);
        let mut proposal: Proposal = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(ContractError::ProposalNotFound)?;

        if proposal.executed {
            return Err(ContractError::ProposalAlreadyExecuted);
        }

        proposal.vetoed = true;
        env.storage().persistent().set(&key, &proposal);
        Self::bump_persistent_ttl(&env, &key);

        env.events().publish(
            (symbol_short!("multisig"), symbol_short!("vetoed")),
            proposal.id,
        );

        Ok(())
    }

    /// Creates a streaming linear timelock schedule.
    pub fn create_stream(
        env: Env,
        sender: Address,
        recipient: Address,
        total_amount: i128,
        duration_seconds: u64,
    ) -> Result<u64, ContractError> {
        sender.require_auth();
        Self::bump_instance_ttl(&env);

        if total_amount <= 0 {
            return Err(ContractError::ZeroAmount);
        }
        if duration_seconds == 0 {
            return Err(ContractError::InvalidTimelock);
        }

        let mut counter: u64 = env
            .storage()
            .instance()
            .get(&DataKey::StreamCounter)
            .unwrap_or(0);
        counter += 1;
        env.storage().instance().set(&DataKey::StreamCounter, &counter);

        let start_time = env.ledger().timestamp();
        let stop_time = start_time + duration_seconds;

        let stream = StreamSchedule {
            id: counter,
            sender,
            recipient,
            total_amount,
            claimed_amount: 0,
            start_time,
            stop_time,
            is_cancelled: false,
        };

        let key = DataKey::Stream(counter);
        env.storage().persistent().set(&key, &stream);
        Self::bump_persistent_ttl(&env, &key);

        Ok(counter)
    }

    /// Claims vested funds from a streaming timelock schedule.
    pub fn claim_stream(
        env: Env,
        recipient: Address,
        stream_id: u64,
    ) -> Result<i128, ContractError> {
        recipient.require_auth();
        Self::bump_instance_ttl(&env);

        let key = DataKey::Stream(stream_id);
        let mut stream: StreamSchedule = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(ContractError::StreamNotFound)?;

        if recipient != stream.recipient {
            return Err(ContractError::Unauthorized);
        }

        let now = env.ledger().timestamp();
        let vested_amount = if now >= stream.stop_time {
            stream.total_amount
        } else if now <= stream.start_time {
            0
        } else {
            let elapsed = (now - stream.start_time) as u128;
            let duration = (stream.stop_time - stream.start_time) as u128;
            let total = stream.total_amount as u128;
            let product = total.checked_mul(elapsed).unwrap_or(u128::MAX);
            (product / duration) as i128
        };

        let claimable = vested_amount - stream.claimed_amount;
        if claimable <= 0 {
            return Ok(0);
        }

        stream.claimed_amount += claimable;
        env.storage().persistent().set(&key, &stream);
        Self::bump_persistent_ttl(&env, &key);

        env.events().publish(
            (symbol_short!("stream"), symbol_short!("claimed")),
            claimable,
        );

        Ok(claimable)
    }

    /// Cancels a streaming schedule. Can be invoked by stream sender or guardian.
    /// Freezes unvested balance and returns unvested amount.
    pub fn cancel_stream(
        env: Env,
        caller: Address,
        stream_id: u64,
    ) -> Result<i128, ContractError> {
        caller.require_auth();
        Self::bump_instance_ttl(&env);

        let config: TreasuryConfig = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .ok_or(ContractError::NotInitialized)?;

        let key = DataKey::Stream(stream_id);
        let mut stream: StreamSchedule = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(ContractError::StreamNotFound)?;

        if caller != stream.sender && caller != config.guardian {
            return Err(ContractError::Unauthorized);
        }
        if stream.is_cancelled {
            return Err(ContractError::ProposalVetoed);
        }

        let now = env.ledger().timestamp();
        let vested = if now >= stream.stop_time {
            stream.total_amount
        } else if now <= stream.start_time {
            0
        } else {
            let elapsed = (now - stream.start_time) as u128;
            let duration = (stream.stop_time - stream.start_time) as u128;
            let total = stream.total_amount as u128;
            let product = total.checked_mul(elapsed).unwrap_or(u128::MAX);
            (product / duration) as i128
        };

        let unvested = stream.total_amount.saturating_sub(vested);
        stream.total_amount = vested;
        stream.stop_time = now;
        stream.is_cancelled = true;

        env.storage().persistent().set(&key, &stream);
        Self::bump_persistent_ttl(&env, &key);

        env.events().publish(
            (symbol_short!("stream"), symbol_short!("cancelled")),
            stream_id,
        );

        Ok(unvested)
    }
}

impl MultisigTimelockContract {
    /// Extends instance storage TTL to prevent state eviction.
    pub fn bump_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }

    /// Extends persistent storage entry TTL.
    pub fn bump_persistent_ttl(env: &Env, key: &DataKey) {
        env.storage()
            .persistent()
            .extend_ttl(key, PERSISTENT_LIFETIME_THRESHOLD, PERSISTENT_BUMP_AMOUNT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    #[test]
    fn test_multisig_timelock_and_veto_lifecycle() {
        let env = Env::default();
        let contract_id = env.register(MultisigTimelockContract, ());
        let client = MultisigTimelockContractClient::new(&env, &contract_id);

        let signer1 = Address::generate(&env);
        let signer2 = Address::generate(&env);
        let signer3 = Address::generate(&env);
        let guardian = Address::generate(&env);
        let recipient = Address::generate(&env);

        let mut signers = Vec::new(&env);
        signers.push_back(signer1.clone());
        signers.push_back(signer2.clone());
        signers.push_back(signer3.clone());

        // Initialize with threshold 2-of-3 and 100 seconds timelock
        client.initialize(&signers, &2, &100, &guardian);

        // Propose transaction by signer1
        env.ledger().set_timestamp(1_000);
        let proposal_id = client.mock_all_auths().propose_transaction(
            &signer1,
            &recipient,
            &10_000,
        );
        assert_eq!(proposal_id, 1);

        // Signer 2 approves
        client.mock_all_auths().approve_proposal(&signer2, &proposal_id);

        // Try to execute before timelock (eta is 1_100, now is 1_000) -> should fail
        let early_res = client.try_execute_proposal(&proposal_id);
        assert!(early_res.is_err());

        // Advance ledger timestamp to 1_105
        env.ledger().set_timestamp(1_105);
        client.execute_proposal(&proposal_id);

        // Test Emergency Veto on a second proposal
        let prop_2 = client.mock_all_auths().propose_transaction(
            &signer1,
            &recipient,
            &50_000,
        );
        client.mock_all_auths().emergency_veto(&guardian, &prop_2);

        // Vetoed proposal cannot be executed
        env.ledger().set_timestamp(2_000);
        let veto_exec_res = client.try_execute_proposal(&prop_2);
        assert!(veto_exec_res.is_err());
    }

    #[test]
    fn test_streaming_timelock_vesting() {
        let env = Env::default();
        let contract_id = env.register(MultisigTimelockContract, ());
        let client = MultisigTimelockContractClient::new(&env, &contract_id);

        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let guardian = Address::generate(&env);

        let mut signers = Vec::new(&env);
        signers.push_back(sender.clone());
        client.initialize(&signers, &1, &10, &guardian);

        env.ledger().set_timestamp(1_000);
        // Create 100-second stream for 1,000 tokens
        let stream_id = client.mock_all_auths().create_stream(
            &sender,
            &recipient,
            &1_000,
            &100,
        );

        // Halfway through (50 seconds)
        env.ledger().set_timestamp(1_050);
        let claimed_1 = client.mock_all_auths().claim_stream(&recipient, &stream_id);
        assert_eq!(claimed_1, 500);

        // Full duration (100 seconds)
        env.ledger().set_timestamp(1_100);
        let claimed_2 = client.mock_all_auths().claim_stream(&recipient, &stream_id);
        assert_eq!(claimed_2, 500); // Remaining 500 claimed
    }

    #[test]
    fn test_stream_cancellation() {
        let env = Env::default();
        let contract_id = env.register(MultisigTimelockContract, ());
        let client = MultisigTimelockContractClient::new(&env, &contract_id);

        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let guardian = Address::generate(&env);

        let mut signers = Vec::new(&env);
        signers.push_back(sender.clone());
        client.initialize(&signers, &1, &10, &guardian);

        env.ledger().set_timestamp(1_000);
        let stream_id = client.mock_all_auths().create_stream(&sender, &recipient, &10_000, &100);

        // Cancel after 25% (25s) by sender
        env.ledger().set_timestamp(1_025);
        let unvested = client.mock_all_auths().cancel_stream(&sender, &stream_id);
        assert_eq!(unvested, 7_500);

        // Recipient can still claim the vested 2,500
        let claimed = client.mock_all_auths().claim_stream(&recipient, &stream_id);
        assert_eq!(claimed, 2_500);

        // Subsequent claim yields 0
        let claimed_sub = client.mock_all_auths().claim_stream(&recipient, &stream_id);
        assert_eq!(claimed_sub, 0);
    }
}
