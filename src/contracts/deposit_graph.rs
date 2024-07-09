// src/contracts/deposit_graph.rs

use ethers::prelude::*;

abigen!(
    DepositGraph,
    "../artifacts/contracts/DepositGraph.sol/DepositGraph.json",
    event_derives(serde::Serialize, serde::Deserialize)
);

// Explicitly export the types we need
pub use self::deposit_graph::{
    WithdrawalRequestedFilter,
    SharesUpdatedFilter,
    UserSignedUpFilter,
    DepositGraph,
};