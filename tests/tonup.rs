use std::ops::Neg;
use std::thread;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use common::new_mainnet_client;
use lazy_static::lazy_static;
use num_bigint::{BigInt, BigUint};
use tokio::{self};
use tokio_test::assert_ok;
use tonlib::address::TonAddress;
use tonlib::cell::{ArcCell, BagOfCells, Cell, StateInit, StateInitBuilder};
use tonlib::client::{self, TonClient, TonClientInterface};
use tonlib::contract::{
    JettonData, JettonMasterContract, TonContract, TonContractError, TonContractFactory,
    TonContractInterface, TonContractState,
};
use tonlib::emulator::{TvmEmulator, TvmEmulatorC7Builder};
use tonlib::message::JettonTransferMessage;
use tonlib::meta::MetaDataContent;
use tonlib::mnemonic::Mnemonic;
use tonlib::tl::{InternalTransactionId, RawFullAccountState};
use tonlib::types::{TvmStackEntry, TvmSuccess};
use tonlib::wallet::{TonWallet, WalletVersion};
mod common;
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct PoolData {
    pub reserve0: BigUint,
    pub reserve1: BigUint,
    pub token0_address: TonAddress,
    pub token1_address: TonAddress,
    pub lp_fee: i32,
    pub protocol_fee: i32,
    pub ref_fee: i32,
    pub protocol_fee_address: TonAddress,
    pub collected_token0_protocol_fee: BigUint,
    pub collected_token1_protocol_fee: BigUint,
}

#[async_trait]
pub trait PoolContract: TonContractInterface {
    async fn get_pool_data(&self) -> anyhow::Result<PoolData> {
        let res = assert_ok!(self.run_get_method("get_pool_data", &Vec::new()).await);
        if res.stack.len() == 10 {
            let pool_data = PoolData {
                reserve0: assert_ok!(res.stack[0].get_biguint()),
                reserve1: assert_ok!(res.stack[1].get_biguint()),
                token0_address: assert_ok!(res.stack[2].get_address()),
                token1_address: assert_ok!(res.stack[3].get_address()),
                lp_fee: assert_ok!(res.stack[4].get_i64()) as i32,
                protocol_fee: assert_ok!(res.stack[5].get_i64()) as i32,
                ref_fee: assert_ok!(res.stack[6].get_i64()) as i32,
                protocol_fee_address: assert_ok!(res.stack[7].get_address()),
                collected_token0_protocol_fee: assert_ok!(res.stack[8].get_biguint()),
                collected_token1_protocol_fee: assert_ok!(res.stack[9].get_biguint()),
            };
            Ok(pool_data)
        } else {
            Err(anyhow!(
                "Invalid result size: {}, expected 10",
                res.stack.len()
            ))
        }
    }

    async fn invalid_method(&self) -> Result<TvmSuccess, TonContractError> {
        self.run_get_method("invalid_method", &Vec::new()).await
    }
}

impl<T> PoolContract for T where T: TonContractInterface {}

#[tokio::test]
async fn state_clone_works() {
    common::init_logging();
    let client = common::new_archive_mainnet_client().await;
    let factory: TonContractFactory =
        assert_ok!(TonContractFactory::builder(&client).build().await);
    let contract = factory.get_contract(&assert_ok!(
        // "EQAxN5aREN_WF-He1uKeQsalDxobF3kz2u-5U69x_RAzK3sQ".parse() //b
        // "EQDqaWBAiPL8d8HMhQPKu55D6iaHY16Rsu15zzdMaRCtlsHm".parse() //c
        // "EQBBuXhxtQGO3mjIXhCoHSYpksP_s_iBLLHk0f9J7wG6LFza".parse() //d
        // "EQDDwrHiXTHt3XwW_X3JS5O6PhYn6fSRlI6YB-lkDMeKlc_V".parse() //e
        "UQCqfGGPYqDhFC2-6FcAldnWwkKHcs_Ez9RoXvJF_in3y_ll".parse() //a
    ));
    let tx_id: &InternalTransactionId = &InternalTransactionId::from_lt_hash(
        // 46643930000001 as i64,
        // "3233504acd607771067010628f2f2f80088d957e1a899a9e436b429735a0e3be",
        // 46643925000001 as i64,
        // "923718b6d1c64cae3564ef6b4d3295bf28f63a83cfc299a1a6d5a50fd3654ff9",
        // 46643937000001 as i64,
        // "04a9742e2419b53a37389ab656bbfa29c13b9d5a55570748141c69ea0fe897bf",
        // 46643940000001 as i64,
        // "9891f4aea497c6714e3c863f77c52d1757e9b1d832b82361af9472bb873e3911",
        46643918000001 as i64,
        "d79d3604cc439b830a850b074fcd4c65e214433d1d3bec12d3bf2d39524b2cd9",
    )
    .unwrap();
    let state1: RawFullAccountState =
        assert_ok!(contract.get_account_state_by_transaction(tx_id).await);
    log::info!("balance: {:?}", state1.balance);
    log::info!("data {:?}", state1.data);
    log::info!("block_id: {:?}", state1.block_id);
    log::info!("frozen_hash: {:?}", state1.frozen_hash);
    log::info!("code {:?}", state1.code);
}
#[tokio::test]
async fn test_run() {
    common::init_logging();

    let contract_addrs: Vec<TonAddress> = vec![
        "UQCqfGGPYqDhFC2-6FcAldnWwkKHcs_Ez9RoXvJF_in3y_ll", //a
        "EQAxN5aREN_WF-He1uKeQsalDxobF3kz2u-5U69x_RAzK3sQ", //b
        "EQDqaWBAiPL8d8HMhQPKu55D6iaHY16Rsu15zzdMaRCtlsHm", //c
        "EQBBuXhxtQGO3mjIXhCoHSYpksP_s_iBLLHk0f9J7wG6LFza", //d
        "EQDDwrHiXTHt3XwW_X3JS5O6PhYn6fSRlI6YB-lkDMeKlc_V", //e
    ]
    .into_iter()
    .map(|e| TonAddress::from_base64_url(e).unwrap())
    .collect();
    log::info!("{:?}", contract_addrs);
    let tx_ids: Vec<InternalTransactionId> = vec![
        (
            46643918000001 as i64,
            "d79d3604cc439b830a850b074fcd4c65e214433d1d3bec12d3bf2d39524b2cd9",
        ),
        (
            46643930000001 as i64,
            "3233504acd607771067010628f2f2f80088d957e1a899a9e436b429735a0e3be",
        ),
        (
            46643925000001 as i64,
            "923718b6d1c64cae3564ef6b4d3295bf28f63a83cfc299a1a6d5a50fd3654ff9",
        ),
        (
            46643937000001 as i64,
            "04a9742e2419b53a37389ab656bbfa29c13b9d5a55570748141c69ea0fe897bf",
        ),
        (
            46643940000001 as i64,
            "9891f4aea497c6714e3c863f77c52d1757e9b1d832b82361af9472bb873e3911",
        ),
    ]
    .into_iter()
    .map(|(a, b)| InternalTransactionId::from_lt_hash(a, b).unwrap())
    .collect();
    let codes: Vec<BagOfCells> = vec![
        include_str!("../resources/tonup/a.code"),
        include_str!("../resources/tonup/b.code"),
        include_str!("../resources/tonup/c.code"),
        include_str!("../resources/tonup/d.code"),
        include_str!("../resources/tonup/e.code"),
    ]
    .into_iter()
    .map(|e| assert_ok!(BagOfCells::parse_base64(e)))
    .collect::<Vec<BagOfCells>>();
    let data: Vec<BagOfCells> = vec![
        include_str!("../resources/tonup/a.data"),
        include_str!("../resources/tonup/b.data"),
        include_str!("../resources/tonup/c.data"),
        include_str!("../resources/tonup/d.data"),
        include_str!("../resources/tonup/e.data"),
    ]
    .into_iter()
    .map(|e| assert_ok!(BagOfCells::parse_base64(e)))
    .collect();

    let client: TonClient = common::new_archive_mainnet_client().await;
    let factory: TonContractFactory = TonContractFactory::builder(&client).build().await.unwrap();
    let contract = factory.get_contract(&contract_addrs[0]);
    let account_state: RawFullAccountState = contract
        .get_account_state_by_transaction(&tx_ids[0])
        .await
        .unwrap();
    let code: &[u8] = &account_state.code;
}

fn get_address(code: BagOfCells, data: BagOfCells) -> TonAddress {
    let tmp_code: &ArcCell = code.single_root().unwrap();
    let tmp_data: &ArcCell = data.single_root().unwrap();
    // let state_init: Cell = StateInitBuilder::new(tmp_code, tmp_data)
    //     .with_split_depth(false)
    //     .with_tick_tock(false)
    //     .with_library(false)
    //     .build()
    //     .unwrap();
    let vec = StateInit::create_account_id(tmp_code, tmp_data).unwrap();
    let mut arr = [0u8; 32];
    let hash_part = arr.copy_from_slice(&vec);

    TonAddress::new(0, hash_part)

    // TonAddress::null()
}
