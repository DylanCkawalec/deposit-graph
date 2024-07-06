use ethers::prelude::*;
/*
this needs to be referencing DepositGraph/build/contracts/DepositGraph.json from this file in `DepositGraph/deposit-graph/src/contracts/deposit_graph.rs` 
*/
abigen!(
    DepositGraph,
    "../build/contracts/DepositGraph.json",
    event_derives(serde::Serialize, serde::Deserialize)
);
