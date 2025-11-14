//!
//! Lancer Protocol Contract - Arbitrum Stylus Implementation
//!
//! This contract implements the dispute resolution system with judge voting
//! and reputation management.
//!
//! Original Solidity contract converted to Rust for Arbitrum Stylus
//! @author 0xDarioSanchez
//!
//! Note: this code is a conversion and has not been audited.
//!

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use alloy_sol_types::sol;
use stylus_sdk::{
    alloy_primitives::{Address, U256, U64, U32, U8, I8, FixedBytes},
    prelude::*,
    crypto::keccak,
    // call::Call, // COMMENTED OUT - not needed without USDC transfers
    function_selector,
};
use stylus_sdk::stylus_core::{log, calls::errors::Error as CallError};

// ====================================
//          STORAGE STRUCTS          
// ====================================

sol_storage! {
    #[entrypoint]
    pub struct ProtocolContract {
        address owner;
        address usdc_token;
        
        uint256 contract_balance;
        uint64 dispute_count;
        uint8 number_of_votes;
        uint256 dispute_price;
        
        mapping(address => Judge) judges;
        mapping(uint64 => Dispute) disputes;
    }
    
    pub struct Judge {
        address judge_address;
        uint256 balance;
        int8 reputation;
    }
    
    pub struct Dispute {
        uint32 dispute_id;
        address contract_address;
        address requester;
        address beneficiary;
        mapping(uint256 => address) able_to_vote;
        uint256 able_to_vote_count;
        mapping(uint256 => address) voters;
        uint256 voters_count;
        mapping(uint256 => bytes32) vote_commits; // hash(vote, secret)
        mapping(uint256 => bool) revealed;
        mapping(uint256 => bool) vote_plain;      // real vote revealed later
        uint256 commits_count;
        uint256 reveals_count;
        uint8 votes_for;
        uint8 votes_against;
        bool waiting_for_judges;
        bool is_open;
        bool resolved;
    }
}

// ====================================
//             EVENTS          
// ====================================

sol! {
    event JudgeRegistered(address indexed judge);
    event DisputeCreated(uint256 indexed dispute_id, address indexed requester, address indexed contract_address);
    event DisputeResolved(uint256 indexed dispute_id, address winner);
    
    error NotOwner();
    error AlreadyRegistered();
    error NotAJudge();
    error NotTheRequester();
    error ProofCannotBeEmpty();
    error NotTheBeneficiary();
    error NotEnoughReputation();
    error JudgesAlreadyAssigned();
    error JudgeAlreadyRegistered();
    error JudgeNotAllowedToVote();
    error DisputeAlreadyResolved();
    error DisputeNotOpen();
    error JudgeAlreadyVoted();
    error MustBeGreaterThanZero();
    error DisputeNotResolvedYet();
    error NoBalanceToWithdraw();
    error NoUSDCToWithdraw();
}

// ====================================
//          ERROR TYPES          
// ====================================

#[derive(SolidityError)]
pub enum ProtocolError {
    NotOwner(NotOwner),
    AlreadyRegistered(AlreadyRegistered),
    NotAJudge(NotAJudge),
    NotTheRequester(NotTheRequester),
    ProofCannotBeEmpty(ProofCannotBeEmpty),
    NotTheBeneficiary(NotTheBeneficiary),
    NotEnoughReputation(NotEnoughReputation),
    JudgesAlreadyAssigned(JudgesAlreadyAssigned),
    JudgeAlreadyRegistered(JudgeAlreadyRegistered),
    JudgeNotAllowedToVote(JudgeNotAllowedToVote),
    DisputeAlreadyResolved(DisputeAlreadyResolved),
    DisputeNotOpen(DisputeNotOpen),
    JudgeAlreadyVoted(JudgeAlreadyVoted),
    MustBeGreaterThanZero(MustBeGreaterThanZero),
    DisputeNotResolvedYet(DisputeNotResolvedYet),
    NoBalanceToWithdraw(NoBalanceToWithdraw),
    NoUSDCToWithdraw(NoUSDCToWithdraw),
    CallFailed(CallFailed),
}

sol! {
    error CallFailed();
}

// Implement From for CallError (new API)
impl From<CallError> for ProtocolError {
    fn from(_error: CallError) -> Self {
        ProtocolError::CallFailed(CallFailed {})
    }
}

// Implement From for old stylus_sdk::call::Error (deprecated but still used by ERC20)
impl From<stylus_sdk::call::Error> for ProtocolError {
    fn from(_error: stylus_sdk::call::Error) -> Self {
        ProtocolError::CallFailed(CallFailed {})
    }
}

// ====================================
//        CONSTANTS          
// ====================================

const USDC_DECIMALS: u8 = 6;

// ====================================
//      EXTERNAL INTERFACE CALLS          
// ====================================

sol_interface! {
    interface IERC20 {
        function transferFrom(address from, address to, uint256 amount) external returns (bool);
        function transfer(address to, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
    }
}

// ====================================
//        IMPLEMENTATION          
// ====================================

#[public]
impl ProtocolContract {
    
    // ====================================
    //           CONSTRUCTOR          
    // ====================================
    
    /// Initialize the protocol contract
    pub fn init(
        &mut self,
        owner: Address,
        usdc: Address,
    ) -> Result<(), ProtocolError> {
        self.owner.set(owner);
        self.usdc_token.set(usdc);
        self.contract_balance.set(U256::ZERO);
        self.dispute_count.set(U64::from(1));
        self.number_of_votes.set(U8::from(5));
        
        // 50 USDC with 6 decimals
        let dispute_price = U256::from(50u64) * U256::from(10u64.pow(USDC_DECIMALS as u32));
        self.dispute_price.set(dispute_price);
        
        Ok(())
    }
    
    // ====================================
    //        ONLY-OWNER FUNCTIONS          
    // ====================================
    
    /// Update the number of votes required to resolve a dispute
    pub fn update_number_of_votes(&mut self, new_number: u8) -> Result<(), ProtocolError> {
        if self.__stylus_host.msg_sender() != self.owner.get() {
            return Err(ProtocolError::NotOwner(NotOwner {}));
        }
        
        if new_number == 0 {
            return Err(ProtocolError::MustBeGreaterThanZero(MustBeGreaterThanZero {}));
        }
        
        self.number_of_votes.set(U8::from(new_number));
        Ok(())
    }
    
    /// Withdraw available USDC (excludes judge rewards)
    pub fn withdraw(&mut self) -> Result<(), ProtocolError> {
        let sender = self.__stylus_host.msg_sender();
        if sender != self.owner.get() {
            return Err(ProtocolError::NotOwner(NotOwner {}));
        }
        
        // COMMENTED OUT FOR TESTING - USDC transfer logic
        // let usdc = self.usdc_token.get();
        // let contract_addr = self.__stylus_host.contract_address();
        // let token = IERC20::new(usdc);
        // let call = Call::new_in(self);
        // let balance = token.balance_of(call, contract_addr)?;
        
        // let contract_balance = self.contract_balance.get();
        
        // if balance <= contract_balance {
        //     return Err(ProtocolError::NoUSDCToWithdraw(NoUSDCToWithdraw {}));
        // }
        
        // let amount_to_withdraw = balance - contract_balance;
        
        // Reset contract balance
        self.contract_balance.set(U256::ZERO);
        
        // Transfer to owner
        // let token2 = IERC20::new(usdc);
        // let call2 = Call::new_in(self);
        // let success = token2.transfer(call2, sender, amount_to_withdraw)?;
        
        // if !success {
        //     return Err(ProtocolError::CallFailed(CallFailed {}));
        // }
        
        Ok(())
    }
    
    // ====================================
    //         EXTERNAL FUNCTIONS          
    // ====================================
    
    /// Register as a judge
    pub fn register_as_judge(&mut self) -> Result<(), ProtocolError> {
        let sender = self.__stylus_host.msg_sender();
        let judge = self.judges.get(sender);
        
        if judge.judge_address.get() != Address::ZERO {
            return Err(ProtocolError::AlreadyRegistered(AlreadyRegistered {}));
        }
        
        let mut new_judge = self.judges.setter(sender);
        new_judge.judge_address.set(sender);
        new_judge.balance.set(U256::ZERO);
        new_judge.reputation.set(I8::ZERO);
        
        log(&self.__stylus_host, JudgeRegistered { judge: sender });
        
        Ok(())
    }
    
    /// Create a dispute (called by Marketplace contract)
    pub fn create_dispute(
        &mut self,
        deal_id: u64,
        requester: Address,
        _proof: String,
    ) -> Result<(), ProtocolError> {
        let dispute_id = self.dispute_count.get();
        let dispute_id_u64 = u64::from_le_bytes(dispute_id.to_le_bytes());
        
        let mut dispute = self.disputes.setter(dispute_id);
        dispute.dispute_id.set(U32::from(deal_id));
        dispute.requester.set(requester);
        dispute.beneficiary.set(Address::ZERO); // TODO: Get from marketplace
        dispute.contract_address.set(self.__stylus_host.msg_sender());
        dispute.waiting_for_judges.set(true);
        dispute.is_open.set(false);
        dispute.resolved.set(false);
        dispute.votes_for.set(U8::ZERO);
        dispute.votes_against.set(U8::ZERO);
        dispute.able_to_vote_count.set(U256::ZERO);
        dispute.voters_count.set(U256::ZERO);
        
        log(&self.__stylus_host, DisputeCreated {
            dispute_id: U256::from(dispute_id_u64),
            requester,
            contract_address: self.__stylus_host.msg_sender(),
        });
        
        // Increment counter
        let current_counter = self.dispute_count.get();
        self.dispute_count.set(current_counter + U64::from(1));
        
        Ok(())
    }
    
    /// Create a dispute directly (for testing without marketplace)
    /// Caller must have approved Protocol to spend dispute_price USDC
    pub fn create_dispute_direct(
        &mut self,
        deal_id: u64,
        beneficiary: Address,
        _proof: String,
    ) -> Result<(), ProtocolError> {
        let sender = self.__stylus_host.msg_sender();
        
        // COMMENTED OUT FOR TESTING - USDC transfer logic
        // Transfer dispute fee from sender to this contract
        // let usdc = self.usdc_token.get();
        // let dispute_price = self.dispute_price.get();
        let contract_addr = self.__stylus_host.contract_address();
        
        // let token = IERC20::new(usdc);
        // let call = Call::new_in(self);
        
        // Transfer USDC from sender to protocol
        // token.transfer_from(call, sender, contract_addr, dispute_price)?;
        
        // Create dispute
        let dispute_id = self.dispute_count.get();
        let dispute_id_u64 = u64::from_le_bytes(dispute_id.to_le_bytes());
        
        let mut dispute = self.disputes.setter(dispute_id);
        dispute.dispute_id.set(U32::from(deal_id));
        dispute.requester.set(sender);
        dispute.beneficiary.set(beneficiary);
        dispute.contract_address.set(contract_addr);
        dispute.waiting_for_judges.set(true);
        dispute.is_open.set(false);
        dispute.resolved.set(false);
        dispute.votes_for.set(U8::ZERO);
        dispute.votes_against.set(U8::ZERO);
        dispute.able_to_vote_count.set(U256::ZERO);
        dispute.voters_count.set(U256::ZERO);
        dispute.commits_count.set(U256::ZERO);
        dispute.reveals_count.set(U256::ZERO);
        
        log(&self.__stylus_host, DisputeCreated {
            dispute_id: U256::from(dispute_id_u64),
            requester: sender,
            contract_address: contract_addr,
        });
        
        // Increment counter
        let current_counter = self.dispute_count.get();
        self.dispute_count.set(current_counter + U64::from(1));
        
        Ok(())
    }
    
    /// Update dispute proofs for payer
    pub fn update_dispute_for_payer(
        &mut self,
        dispute_id: u64,
        requester: Address,
        _proof: String,
    ) -> Result<(), ProtocolError> {
        let dispute = self.disputes.get(U64::from(dispute_id));
        
        if dispute.requester.get() != requester {
            return Err(ProtocolError::NotTheRequester(NotTheRequester {}));
        }
        
        if _proof.is_empty() {
            return Err(ProtocolError::ProofCannotBeEmpty(ProofCannotBeEmpty {}));
        }
        
        if dispute.resolved.get() {
            return Err(ProtocolError::DisputeAlreadyResolved(DisputeAlreadyResolved {}));
        }
        
        // Note: In production, you'd store the proof in storage or emit an event
        // For now, we just validate the inputs
        
        Ok(())
    }
    
    /// Update dispute proofs for beneficiary
    pub fn update_dispute_for_beneficiary(
        &mut self,
        dispute_id: u64,
        beneficiary: Address,
        _proof: String,
    ) -> Result<(), ProtocolError> {
        let dispute = self.disputes.get(U64::from(dispute_id));
        
        if dispute.beneficiary.get() != beneficiary {
            return Err(ProtocolError::NotTheBeneficiary(NotTheBeneficiary {}));
        }
        
        if _proof.is_empty() {
            return Err(ProtocolError::ProofCannotBeEmpty(ProofCannotBeEmpty {}));
        }
        
        if dispute.resolved.get() {
            return Err(ProtocolError::DisputeAlreadyResolved(DisputeAlreadyResolved {}));
        }
        
        // Note: Store proof or emit event in production
        
        Ok(())
    }
    
    /// Register to vote on a dispute
    pub fn register_to_vote(&mut self, dispute_id: u64) -> Result<(), ProtocolError> {
        let sender = self.__stylus_host.msg_sender();
        
        // SIMPLIFIED FOR TESTING - Just add to able_to_vote list
        let mut dispute_mut = self.disputes.setter(U64::from(dispute_id));
        let current_count = dispute_mut.able_to_vote_count.get();
        dispute_mut.able_to_vote.setter(current_count).set(sender);
        dispute_mut.able_to_vote_count.set(current_count + U256::from(1u64));
        
        // Open dispute when we have 5 judges
        if current_count + U256::from(1u64) >= U256::from(5u64) {
            dispute_mut.waiting_for_judges.set(false);
            dispute_mut.is_open.set(true);
        }
        
        Ok(())
    }
    
    // /// Vote on a dispute
    // pub fn vote(&mut self, dispute_id: u64, support: bool) -> Result<(), ProtocolError> {
    //     let sender = msg::sender();
    //     let dispute = self.disputes.get(U64::from(dispute_id));
        
    //     if dispute.resolved.get() {
    //         return Err(ProtocolError::DisputeAlreadyResolved(DisputeAlreadyResolved {}));
    //     }
        
    //     if !dispute.is_open.get() {
    //         return Err(ProtocolError::DisputeNotOpen(DisputeNotOpen {}));
    //     }
        
    //     // Check if judge is able to vote
    //     let mut found = false;
    //     let able_count = dispute.able_to_vote_count.get();
    //     for i in 0..able_count.as_limbs()[0] {
    //         let judge_addr = dispute.able_to_vote.get(U256::from(i));
    //         if judge_addr == sender {
    //             found = true;
    //             break;
    //         }
    //     }
        
    //     if !found {
    //         return Err(ProtocolError::JudgeNotAllowedToVote(JudgeNotAllowedToVote {}));
    //     }
        
    //     // Check if already voted
    //     let voters_count = dispute.voters_count.get();
    //     for i in 0..voters_count.as_limbs()[0] {
    //         let voter = dispute.voters.get(U256::from(i));
    //         if voter == sender {
    //             return Err(ProtocolError::JudgeAlreadyVoted(JudgeAlreadyVoted {}));
    //         }
    //     }
        
    //     // Record vote
    //     let mut dispute_mut = self.disputes.setter(U64::from(dispute_id));
    //     let current_voters = dispute_mut.voters_count.get();
    //     dispute_mut.voters.setter(current_voters).set(sender);
    //     dispute_mut.votes.setter(current_voters).set(support);
    //     let new_voters_count = current_voters + U256::from(1u64);
    //     dispute_mut.voters_count.set(new_voters_count);
        
    //     if support {
    //         let current_for = dispute_mut.votes_for.get();
    //         dispute_mut.votes_for.set(current_for + U8::from(1));
    //     } else {
    //         let current_against = dispute_mut.votes_against.get();
    //         dispute_mut.votes_against.set(current_against + U8::from(1));
    //     }
        
    //     // Check if all votes are in
    //     let required_votes = self.number_of_votes.get();
    //     let required_votes_u64 = u64::from_le_bytes(required_votes.to_le_bytes());
        
    //     if new_voters_count == U256::from(required_votes_u64) {
    //         dispute_mut.is_open.set(false);
    //         dispute_mut.resolved.set(true);
            
    //         let votes_for = u8::from_le_bytes(dispute_mut.votes_for.get().to_le_bytes());
    //         let votes_against = u8::from_le_bytes(dispute_mut.votes_against.get().to_le_bytes());
            
    //         let dispute_price = self.dispute_price.get();
    //         let prize = dispute_price / U256::from(required_votes_u64);
            
    //         let requester = dispute_mut.requester.get();
    //         let beneficiary = dispute_mut.beneficiary.get();
            
    //         // Distribute rewards and update reputation
    //         if votes_for > votes_against {
    //             // Requester wins
    //             for i in 0..new_voters_count.as_limbs()[0] {
    //                 let voter = dispute_mut.voters.get(U256::from(i));
    //                 let vote = dispute_mut.votes.get(U256::from(i));
                    
    //                 let mut judge = self.judges.setter(voter);
    //                 let current_rep = judge.reputation.get();
                    
    //                 if vote {
    //                     // Voted for winner
    //                     judge.reputation.set(current_rep + I8::from_le_bytes([1, 0, 0, 0, 0, 0, 0, 0]));
    //                     let current_balance = judge.balance.get();
    //                     judge.balance.set(current_balance + prize);
    //                 } else {
    //                     // Voted for loser
    //                     judge.reputation.set(current_rep - I8::from_le_bytes([1, 0, 0, 0, 0, 0, 0, 0]));
    //                 }
    //             }
                
    //             // Contract keeps losing votes' prizes
    //             let contract_reward = prize * U256::from(votes_against as u64);
    //             let current_contract_balance = self.contract_balance.get();
    //             self.contract_balance.set(current_contract_balance + contract_reward);
                
    //             evm::log(DisputeResolved {
    //                 dispute_id: U256::from(dispute_id),
    //                 winner: requester,
    //             });
    //         } else {
    //             // Beneficiary wins
    //             for i in 0..new_voters_count.as_limbs()[0] {
    //                 let voter = dispute_mut.voters.get(U256::from(i));
    //                 let vote = dispute_mut.votes.get(U256::from(i));
                    
    //                 let mut judge = self.judges.setter(voter);
    //                 let current_rep = judge.reputation.get();
                    
    //                 if !vote {
    //                     // Voted for winner
    //                     judge.reputation.set(current_rep + I8::from_le_bytes([1, 0, 0, 0, 0, 0, 0, 0]));
    //                     let current_balance = judge.balance.get();
    //                     judge.balance.set(current_balance + prize);
    //                 } else {
    //                     // Voted for loser
    //                     judge.reputation.set(current_rep - I8::from_le_bytes([1, 0, 0, 0, 0, 0, 0, 0]));
    //                 }
    //             }
                
    //             // Contract keeps losing votes' prizes
    //             let contract_reward = prize * U256::from(votes_for as u64);
    //             let current_contract_balance = self.contract_balance.get();
    //             self.contract_balance.set(current_contract_balance + contract_reward);
                
    //             evm::log(DisputeResolved {
    //                 dispute_id: U256::from(dispute_id),
    //                 winner: beneficiary,
    //             });
    //         }
    //     }
        
    //     Ok(())
    // }
    

    pub fn commit_vote(&mut self, dispute_id: u64, commit_hash: FixedBytes<32>) -> Result<(), ProtocolError> {
        let sender = self.__stylus_host.msg_sender();
        let mut dispute = self.disputes.setter(U64::from(dispute_id));

        // SIMPLIFIED FOR TESTING - Skip all validation
        let commits = dispute.commits_count.get();
        
        // Store commit
        dispute.voters.setter(commits).set(sender);
        dispute.vote_commits.setter(commits).set(commit_hash);
        dispute.commits_count.set(commits + U256::from(1u64));

        Ok(())
    }


    /// Reveal a single judge's vote (called by each judge individually)
    pub fn reveal_votes(
        &mut self,
        dispute_id: u64,
        vote: bool,
        _secret: Vec<u8>
    ) -> Result<(), ProtocolError> {
        let sender = self.__stylus_host.msg_sender();
        let mut dispute = self.disputes.setter(U64::from(dispute_id));

        // SIMPLIFIED FOR TESTING - Skip all validation
        // Find the judge's commit index
        let commit_count = dispute.commits_count.get();
        let mut judge_index: Option<u64> = None;
        
        for i in 0..commit_count.as_limbs()[0] {
            let voter = dispute.voters.get(U256::from(i));
            if voter == sender {
                judge_index = Some(i);
                break;
            }
        }

        let idx = match judge_index {
            Some(i) => i,
            None => return Err(ProtocolError::JudgeNotAllowedToVote(JudgeNotAllowedToVote {})),
        };

        // Mark as revealed and store the vote
        dispute.revealed.setter(U256::from(idx)).set(true);
        dispute.vote_plain.setter(U256::from(idx)).set(vote);
        
        // Update vote counts
        let current_reveals = dispute.reveals_count.get();
        dispute.reveals_count.set(current_reveals + U256::from(1u64));
        
        if vote {
            let current_for = dispute.votes_for.get();
            dispute.votes_for.set(current_for + U8::from(1u8));
        } else {
            let current_against = dispute.votes_against.get();
            dispute.votes_against.set(current_against + U8::from(1u8));
        }

        // Check if all votes are revealed (hardcode 5 for testing)
        if current_reveals + U256::from(1u64) >= U256::from(5u64) {
            // All votes revealed - resolve the dispute
            dispute.is_open.set(false);
            dispute.resolved.set(true);

            let votes_for = dispute.votes_for.get();
            let votes_against = dispute.votes_against.get();

            let requester = dispute.requester.get();
            let beneficiary = dispute.beneficiary.get();

            if votes_for > votes_against {
                log(&self.__stylus_host, DisputeResolved {
                    dispute_id: U256::from(dispute_id),
                    winner: requester,
                });
            } else {
                log(&self.__stylus_host, DisputeResolved {
                    dispute_id: U256::from(dispute_id),
                    winner: beneficiary,
                });
            }
        }

        Ok(())
    }

    
    /// Get dispute winner (called by Marketplace to execute result)
    /// Returns true if requester (payer) wins, false if beneficiary (seller) wins
    pub fn get_dispute_winner(&self, dispute_id: u64) -> Result<bool, ProtocolError> {
        let dispute = self.disputes.get(U64::from(dispute_id));
        
        if !dispute.resolved.get() {
            return Err(ProtocolError::DisputeNotResolvedYet(DisputeNotResolvedYet {}));
        }
        
        let votes_for = u8::from_le_bytes(dispute.votes_for.get().to_le_bytes());
        let votes_against = u8::from_le_bytes(dispute.votes_against.get().to_le_bytes());
        
        // votes_for means vote for requester/payer
        // votes_against means vote for beneficiary/seller
        // Return true if requester wins (votes_for > votes_against)
        Ok(votes_for > votes_against)
    }
    
    /// Execute dispute result - kept for backward compatibility, delegates to get_dispute_winner
    pub fn execute_dispute_result(&self, dispute_id: u64) -> Result<bool, ProtocolError> {
        self.get_dispute_winner(dispute_id)
    }
    
    /// Judge withdraw their balance
    pub fn judge_withdraw(&mut self) -> Result<(), ProtocolError> {
        let sender = self.__stylus_host.msg_sender();
        let judge = self.judges.get(sender);
        
        if judge.judge_address.get() == Address::ZERO {
            return Err(ProtocolError::NotAJudge(NotAJudge {}));
        }
        
        let balance = judge.balance.get();
        if balance == U256::ZERO {
            return Err(ProtocolError::NoBalanceToWithdraw(NoBalanceToWithdraw {}));
        }
        
        // Reset balance
        let mut judge_mut = self.judges.setter(sender);
        judge_mut.balance.set(U256::ZERO);
        
        // COMMENTED OUT FOR TESTING - USDC transfer logic
        // Transfer USDC
        // let usdc = self.usdc_token.get();
        // let token = IERC20::new(usdc);
        // let call = Call::new_in(self);
        // let success = token.transfer(call, sender, balance)?;
        
        // if !success {
        //     return Err(ProtocolError::CallFailed(CallFailed {}));
        // }
        
        Ok(())
    }
    
    // ====================================
    //        VIEW FUNCTIONS          
    // ====================================
    
    /// Get owner address
    pub fn owner(&self) -> Address {
        self.owner.get()
    }
    
    /// Get dispute count
    pub fn dispute_count(&self) -> u64 {
        u64::from_le_bytes(self.dispute_count.get().to_le_bytes())
    }
    
    /// Get number of votes required
    pub fn number_of_votes(&self) -> u8 {
        u8::from_le_bytes(self.number_of_votes.get().to_le_bytes())
    }
    
    /// Get dispute price
    pub fn dispute_price(&self) -> U256 {
        self.dispute_price.get()
    }
    
    /// Check if dispute is resolved
    pub fn check_if_dispute_is_resolved(&self, dispute_id: u64) -> bool {
        let dispute = self.disputes.get(U64::from(dispute_id));
        dispute.resolved.get()
    }
    
    /// Get judge info
    pub fn get_judge(&self, judge_address: Address) -> (Address, U256, i8) {
        let judge = self.judges.get(judge_address);
        (
            judge.judge_address.get(),
            judge.balance.get(),
            i8::from_le_bytes(judge.reputation.get().to_le_bytes()),
        )
    }
    
    /// Get dispute basic info
    pub fn get_dispute(&self, dispute_id: u64) -> (u32, Address, Address, Address, bool, bool, bool) {
        let dispute = self.disputes.get(U64::from(dispute_id));
        (
            u32::from_le_bytes(dispute.dispute_id.get().to_le_bytes()),
            dispute.contract_address.get(),
            dispute.requester.get(),
            dispute.beneficiary.get(),
            dispute.waiting_for_judges.get(),
            dispute.is_open.get(),
            dispute.resolved.get(),
        )
    }
    
    /// Get dispute vote results
    pub fn get_dispute_votes(&self, dispute_id: u64) -> (u8, u8) {
        let dispute = self.disputes.get(U64::from(dispute_id));
        (
            u8::from_le_bytes(dispute.votes_for.get().to_le_bytes()),
            u8::from_le_bytes(dispute.votes_against.get().to_le_bytes()),
        )
    }
}