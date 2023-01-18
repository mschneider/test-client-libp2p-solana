
use std::{sync::Arc, time::Duration};
#[macro_use] extern crate log;
#[macro_use] extern crate solana_metrics;


mod clique_stage;
use clique_stage::{CliqueStageConfig, CliqueStage};


use crossbeam_channel::{unbounded, tick, select};
use rand::RngCore;
use solana_perf::packet::PACKET_DATA_SIZE;
use solana_sdk::signature::Keypair;


fn main() {
    solana_logger::setup_with_default("info");

    let (clique_outbound_sender, clique_outbound_receiver) = unbounded();
    let (clique_inbound_sender, clique_inbound_receiver) = unbounded();

    info!("clique stage config");
    let config = CliqueStageConfig {
        listen_port: 5678,
        identity_keypair: Arc::new(Keypair::new()),
        exit: Arc::default(),
    };

    
    info!("clique stage");
    let _clique_stage = CliqueStage::new(config, clique_outbound_receiver, clique_inbound_sender);

    info!("timer");
    let timer = tick(Duration::from_secs(1));
    
    info!("select loop");

    loop {
        select! {
            recv(timer) -> _ => {
                let mut packet = [0u8; PACKET_DATA_SIZE];
                for i in 0..6000 {
                    rand::thread_rng().fill_bytes(&mut packet);
                    clique_outbound_sender.send(vec![packet.to_vec()]).expect("outbound send")
                }
            },
            recv(clique_inbound_receiver) -> inbound => {
                match inbound {
                    Ok(packets) => {},
                    Err(e) => error!("clique inbound {}", e),
                }
            }
        }
    }

}
