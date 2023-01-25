use std::{
    net::{SocketAddr, UdpSocket},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::{Builder, JoinHandle},
    time::Duration,
};
#[macro_use]
extern crate log;
#[macro_use]
extern crate solana_metrics;

use solana_core::clique_stage::CliqueStage;

use crossbeam_channel::{select, tick, unbounded};
use rand::RngCore;
use solana_perf::packet::{PacketBatchRecycler, PACKET_DATA_SIZE};
use solana_pubsub_client::pubsub_client::PubsubClient;
use solana_sdk::signature::Keypair;
use solana_streamer::{
    socket::SocketAddrSpace,
    streamer::{self, StreamerReceiveStats},
};

fn slot_subscriber(exit: Arc<AtomicBool>, slot: Arc<AtomicU64>) -> JoinHandle<()> {
    let (_subscription, receiver) =
        PubsubClient::slot_subscribe(&"ws://api.mainnet-beta.solana.com/")
            .expect("subscribe to slot updates");

    Builder::new()
        .name("slotSubscribe".to_string())
        .spawn(move || {
            while !exit.load(Ordering::Relaxed) {
                if let Ok(slot_info) = receiver.recv_timeout(Duration::from_secs(1)) {
                    slot.store(slot_info.slot, Ordering::Relaxed);
                }
            }
        })
        .expect("spawn slotSubscribe")
}

fn main() {
    solana_logger::setup_with_default("info");

    let exit = Arc::new(AtomicBool::default());
    let slot = Arc::new(AtomicU64::default());
    let _slot_subscribe_hdl = slot_subscriber(exit.clone(), slot.clone());

    let clique_inbound_addr = "5.62.126.197:5678".parse().expect("valid inbound addr");
    let clique_inbound_socket =
        Arc::new(UdpSocket::bind(clique_inbound_addr).expect("bind to inbound addr"));
    let (clique_inbound_sender, clique_inbound_receiver) = unbounded();

    let receiver_stats = Arc::new(StreamerReceiveStats::new("shred_receiver"));
    let _shred_receiver_hdl = streamer::receiver(
        clique_inbound_socket,
        exit.clone(),
        clique_inbound_sender,
        PacketBatchRecycler::warmed(100, 1024),
        receiver_stats.clone(),
        1,
        false,
        None,
    );

    let (clique_outbound_sender, clique_outbound_receiver) = unbounded();
    let clique_outbound_sockets = Arc::new(
        (0..8)
            .map(|_| UdpSocket::bind("0.0.0.0:0").expect("bind outbound socket to random port"))
            .collect(),
    );
    let exit = Default::default();
    let identity = Arc::new(Keypair::new());

    let slot_query = move || slot.load(Ordering::Relaxed);
    let _clique_stage = CliqueStage::new(
        clique_outbound_receiver,
        clique_outbound_sockets,
        exit,
        identity,
        clique_inbound_addr,
        SocketAddrSpace::Global,
        slot_query.clone()
    );


    info!("select loop");

    loop {
        select! {
            recv(clique_inbound_receiver) -> maybe_batch => {
                if let Ok(batch) = maybe_batch {
                    clique_outbound_sender.send(batch
                        .iter()
                        .filter_map(|p| p.data(..))
                        .map(|d| Vec::from(d))
                        .collect()).expect("outbound send")
                }
            },

            default(Duration::from_secs(1)) => {
                info!("slot {}", slot_query());
                receiver_stats.report();
            }
        }
    }
}
