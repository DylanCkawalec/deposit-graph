use ethers::prelude::*;

abigen!(
    DepositGraph,
    "../build/contracts/DepositGraph.json",
    event_derives(serde::Serialize, serde::Deserialize)
);
