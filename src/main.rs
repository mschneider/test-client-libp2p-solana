mod clique_stage;
#[macro_use] extern crate log;


use std::{sync::Arc, time::Duration};

use clique_stage::{CliqueStageConfig, CliqueStage};
use crossbeam_channel::{unbounded, tick, select};
use rand::{random};
use simplelog::*;
use solana_sdk::signature::Keypair;


fn main() -> Result<(), String> {
    solana_logger::setup_with_default("info");

    let (clique_outbound_sender, clique_outbound_receiver) = unbounded();
    let (clique_inbound_sender, clique_inbound_receiver) = unbounded();

    info!("clique stage config");
    let config = CliqueStageConfig {
        identity_keypair: Arc::new(Keypair::new()),
        exit: Arc::default(),
    };

    
    info!("clique stage");
    let clique_stage = CliqueStage::new(config, clique_outbound_receiver, clique_inbound_sender);

    info!("timer");
    let timer = tick(Duration::from_secs(1 + random::<u64>() % 16));
    
    info!("select loop");
    loop {
        select! {
            recv(timer) -> _ => {
                info!("clique outbound");
                clique_outbound_sender.send(vec![vec![1, 2, 3]]).expect("outbound send")
            },
            recv(clique_inbound_receiver) -> inbound => {
                match inbound {
                    Ok(packets) => info!("clique inbound {} packets", packets.len()),
                    Err(e) => error!("clique inbound {}", e),
                }
            }
        }
    }

    Ok(())
}
