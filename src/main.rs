use std::{
    net::UdpSocket,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::{sleep, Builder, JoinHandle},
    time::Duration,
};
#[macro_use]
extern crate log;
extern crate solana_metrics;

use clap::Parser;
use crossbeam_channel::{select, unbounded};
use solana_client::rpc_client::RpcClient;
use solana_core::clique_stage::CliqueStage;
use solana_perf::packet::PacketBatchRecycler;
use solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair};
use solana_streamer::{
    socket::SocketAddrSpace,
    streamer::{self, StreamerReceiveStats},
};

/// solana clique stage test client
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Address to bind to
    #[arg(short, long)]
    bind_addr: String,

    /// TCP port for receiving gossip data
    #[arg(short, long, default_value_t = 5678)]
    gossip_port: u16,

    /// UDP port for receiving shred data
    #[arg(short, long, default_value_t = 5679)]
    shred_port: usize,

    /// Number of threads for shred transmission
    #[arg(short, long, default_value_t = 8)]
    num_retransmit_threads: usize,

    /// List of HOST:PORT addresses to connect to
    #[arg(short='p', long, num_args(0..))]
    bootstrap_peers: Option<Vec<String>>,
}

fn slot_poller(exit: Arc<AtomicBool>, slot: Arc<AtomicU64>) -> JoinHandle<()> {
    let rpc = RpcClient::new("https://api.testnet.solana.com");
    let commitment = CommitmentConfig::processed();

    Builder::new()
        .name("slotPoll".to_string())
        .spawn(move || {
            while !exit.load(Ordering::Relaxed) {
                match rpc.get_slot_with_commitment(commitment) {
                    Ok(new_slot) => slot.store(new_slot, Ordering::Relaxed),
                    Err(e) => error!("slot_poller {:?}", e),
                }
                sleep(Duration::from_millis(100));
            }
        })
        .expect("spawn slotSubscribe")
}

fn main() {
    solana_logger::setup_with_default("info");

    let args: Args = Args::parse();
    let exit = Arc::new(AtomicBool::default());
    let slot = Arc::new(AtomicU64::default());
    let _slot_poller_hdl = slot_poller(exit.clone(), slot.clone());
    let slot_query = move || slot.load(Ordering::Relaxed);

    let clique_inbound_addr = format!("{}:{}", args.bind_addr, args.shred_port)
        .parse()
        .expect("valid bind addr and shred port");
    info!("listen for shreds at {:?}", clique_inbound_addr);
    let clique_inbound_socket = UdpSocket::bind(clique_inbound_addr).expect("bind to shred addr");
    let (clique_inbound_sender, clique_inbound_receiver) = unbounded();

    let receiver_stats = Arc::new(StreamerReceiveStats::new("shred_receiver"));
    let _shred_receiver_hdl = streamer::receiver(
        Arc::new(clique_inbound_socket),
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
        (0..args.num_retransmit_threads)
            .map(|_| UdpSocket::bind("0.0.0.0:0").expect("bind outbound socket to random port"))
            .collect(),
    );
    let identity = Arc::new(Keypair::new());

    let bind_addr = format!("{}:{}", args.bind_addr, args.gossip_port)
        .parse()
        .expect("valid bind addr and gossip port");
    info!("listen for gossip at {:?}", bind_addr);

    let bootstrap_peers = args
        .bootstrap_peers
        .unwrap_or_default()
        .iter()
        .map(|peer| peer.parse().expect("valid peer addr"))
        .collect();

    info!("waiting for slot subscription to receive the first slot");
    while slot_query() == 0 {
        std::thread::sleep(Duration::from_millis(100))
    }

    let _clique_stage = CliqueStage::new(
        bind_addr,
        Arc::new(bootstrap_peers),
        clique_outbound_receiver,
        clique_outbound_sockets,
        exit.clone(),
        identity,
        clique_inbound_addr,
        SocketAddrSpace::Global,
        slot_query.clone(),
    );

    info!("client ready at slot {}", slot_query());

    while !exit.load(Ordering::Relaxed) {
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
