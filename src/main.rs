#![feature(slice_as_chunks)]

use std::{time::Duration, collections::hash_map::DefaultHasher, hash::{Hash, Hasher}, error::Error, str::FromStr};

use async_std::io;
use futures::{prelude::*, select};
use libp2p::{
    core, gossipsub, identity, identify, ping, swarm::NetworkBehaviour, swarm::SwarmEvent, Multiaddr, PeerId, Swarm, tcp, Transport, noise, yamux
};
use solana_sdk;

#[async_std::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let solana_keypair = solana_sdk::signature::Keypair::new();
    let mut copy = solana_keypair.secret().as_bytes().clone();
    let secret_key = identity::ed25519::SecretKey::from_bytes(copy)?;
    let local_key = identity::Keypair::Ed25519(secret_key.into());
    let local_peer_id = PeerId::from(local_key.public());
    println!("Local peer id: {local_peer_id}");

    // Set up an encrypted DNS-enabled TCP Transport over the Mplex protocol.
    let transport = tcp::async_io::Transport::default()
    .upgrade(core::upgrade::Version::V1)
    .authenticate(noise::NoiseAuthenticated::xx(&local_key.clone())?)
    .multiplex(yamux::YamuxConfig::default())
    .boxed();

    // let shred = [0u8; 1280];
    // for i in 0..1280 {
    //     shred[i] = i as u8
    // }

    // We create a custom network behaviour that combines Gossipsub and Mdns.
    #[derive(NetworkBehaviour)]
    struct MyBehaviour {
        gossipsub: gossipsub::Gossipsub,
        identify: identify::Behaviour,
        ping: ping::Behaviour,
    }

    /*
    // To content-address message, we can take the embedded signature of the shred and use it as an ID.
    let message_id_fn = |message: &gossipsub::GossipsubMessage| {
        gossipsub::MessageId::from(message.data.as_chunks::<24>().0[0])
    };
    */

        // To content-address message, we can take the hash of message and use it as an ID.
        let message_id_fn = |message: &gossipsub::GossipsubMessage| {
            let mut s = DefaultHasher::new();
            message.data.hash(&mut s);
            gossipsub::MessageId::from(s.finish().to_string())
        };

    // Set a custom gossipsub configuration
    let gossipsub_config = gossipsub::GossipsubConfigBuilder::default()
    .heartbeat_interval(Duration::from_secs(10)) // This is set to aid debugging by not cluttering the log space
    .validation_mode(gossipsub::ValidationMode::Strict) // This sets the kind of message validation. The default is Strict (enforce message signing)
    .message_id_fn(message_id_fn) // content-address messages. No two messages of the same content will be propagated.
    .build()
    .expect("Valid config");

     // build a gossipsub network behaviour
     let mut gossipsub = gossipsub::Gossipsub::new(gossipsub::MessageAuthenticity::Signed(local_key.clone()), gossipsub_config)
     .expect("Correct configuration");


    // Create a Gossipsub topic
    let topic : gossipsub::Topic<gossipsub::topic::Sha256Hash> = gossipsub::Topic::new("test-net");

    // subscribes to our topic
    gossipsub.subscribe(&topic)?;

    let identify = identify::Behaviour::new(identify::Config::new(
        "/ipfs/0.1.0".into(),
        local_key.public(),
    ));

    let ping = ping::Behaviour::new(ping::Config::new());


    // Create a Swarm to manage peers and events
    let mut swarm = {
        let behaviour = MyBehaviour { gossipsub, identify, ping };
        Swarm::with_async_std_executor(transport, behaviour, local_peer_id)
    };


    // Reach out to other nodes if specified
    for to_dial in std::env::args().skip(1) {
        let addr = Multiaddr::from_str(&to_dial)?;
        swarm.dial(addr)?;
        println!("Dialed {to_dial:?}")
    }

    // Read full lines from stdin
    let mut stdin = io::BufReader::new(io::stdin()).lines().fuse();

    // Listen on all interfaces
    swarm.listen_on("/ip4/0.0.0.0/tcp/8765".parse()?)?;

    println!("Enter messages via STDIN and they will be sent to connected peers using Gossipsub");

    // Kick it off
    loop {
        select! {
            line = stdin.select_next_some() => {
                if let Err(e) = swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(topic.clone(), line.expect("Stdin not to close").as_bytes())
                {
                    println!("Publish error: {e:?}");
                }
            },
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("Listening on {address:?}");
                    }
                    SwarmEvent::Behaviour(MyBehaviourEvent::Identify(event)) => {
                        println!("identify: {event:?}");
                    }
                    SwarmEvent::Behaviour(MyBehaviourEvent::Gossipsub(gossipsub::GossipsubEvent::Message {
                        propagation_source: peer_id,
                        message_id: id,
                        message,
                    })) => {
                        println!(
                            "Got message: {} with id: {} from peer: {:?}",
                            String::from_utf8_lossy(&message.data),
                            id,
                            peer_id
                        )
                    }
                    SwarmEvent::Behaviour(MyBehaviourEvent::Ping(event)) => {
                        match event {
                            ping::Event {
                                peer,
                                result: Result::Ok(ping::Success::Ping { rtt }),
                            } => {
                                println!(
                                    "ping: rtt to {} is {} ms",
                                    peer.to_base58(),
                                    rtt.as_millis()
                                );
                            }
                            ping::Event {
                                peer,
                                result: Result::Ok(ping::Success::Pong),
                            } => {
                                println!("ping: pong from {}", peer.to_base58());
                            }
                            ping::Event {
                                peer,
                                result: Result::Err(ping::Failure::Timeout),
                            } => {
                                println!("ping: timeout to {}", peer.to_base58());
                            }
                            ping::Event {
                                peer,
                                result: Result::Err(ping::Failure::Unsupported),
                            } => {
                                println!("ping: {} does not support ping protocol", peer.to_base58());
                            }
                            ping::Event {
                                peer,
                                result: Result::Err(ping::Failure::Other { error }),
                            } => {
                                println!("ping: ping::Failure with {}: {error}", peer.to_base58());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

