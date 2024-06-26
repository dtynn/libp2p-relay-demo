use std::{net::Ipv4Addr, collections::HashSet};
use std::any::type_name_of_val;
use std::collections::HashMap;

use clap::Parser;
use futures::{StreamExt, executor::block_on, FutureExt};
use libp2p::{
    autonat, dcutr, identify, identity::Keypair, multiaddr::Protocol, noise, ping, relay, tcp,
    tcp::tokio::Transport as TokioTcpTransport, yamux, Multiaddr, SwarmBuilder, Transport,
    swarm::{SwarmEvent, ConnectionId}, PeerId, core::{ConnectedPoint, Endpoint}, kad::{self, store::MemoryStore},
};
use tracing::{warn, info, warn_span, debug};
use tracing_subscriber::EnvFilter;

mod behaviour;
mod transport;

pub(crate) use transport::is_holepunch_direct_addr;
use behaviour::{Behaviour, BehaviourEvent};

#[derive(Debug, Parser)]
#[clap(name = "libp2p relay node")]
struct Opt {
    /// Fixed value to generate deterministic peer id
    #[clap(long)]
    seed: u8,

    /// The port used to listen on all interfaces
    #[clap(long)]
    listen_port: u16,

    #[clap(long)]
    connect: Vec<Multiaddr>,

    #[clap(long)]
    peer: Option<Multiaddr>,

    #[clap(long, default_value_t = false)]
    relay_service: bool,

    #[clap(long, default_value_t = false)]
    listen_relayed: bool,

    #[clap(long)]
    dcutr_port: Option<u16>,

    #[clap(long, default_value_t = false)]
    kad: bool,

    #[clap(long)]
    kad_put: Option<String>,

    #[clap(long)]
    kad_get: Option<String>,
}

fn generate_ed25519(secret_key_seed: u8) -> Keypair {
    let mut bytes = [0u8; 32];
    bytes[0] = secret_key_seed;

    Keypair::ed25519_from_bytes(bytes).expect("only errors on wrong length")
}

#[tokio::main]
async fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    let opt = Opt::parse();
    info!("options {:?}", opt);

    let key = generate_ed25519(opt.seed);
    let tcp_cfg = tcp::Config::default();

    let mut swarm = SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_other_transport(|keypair| {
            let tcp_trans = transport::HolePunchTransport::new(tcp_cfg.clone())
                .or_transport(TokioTcpTransport::new(tcp_cfg));

            let tcp_upgraded = {
                let noise = noise::Config::new(keypair)
                    .expect("Signing libp2p-noise static DH keypair failed.");

                tcp_trans
                    .upgrade(libp2p::core::upgrade::Version::V1Lazy)
                    .authenticate(noise)
                    .multiplex(yamux::Config::default())
                    .timeout(std::time::Duration::from_secs(2))
                    .boxed()
            };

            Ok(libp2p::dns::tokio::Transport::system(tcp_upgraded)?.boxed())
        })
        .expect("swarm builder with transport")
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .expect("swarm with relay client")
        .with_behaviour(|key, relay_client| Behaviour {
            kad: opt.kad.then(|| kad::Behaviour::new(key.public().to_peer_id(), MemoryStore::new(key.public().to_peer_id())).into()).into(),
            relay: opt
                .relay_service
                .then(|| relay::Behaviour::new(key.public().to_peer_id(), Default::default()))
                .into(),
            relay_client,
            dcutr: opt
                .dcutr_port
                .map(|_| dcutr::Behaviour::new(key.public().to_peer_id()).into())
                .into(),
            autonat: autonat::Behaviour::new(key.public().to_peer_id(), autonat::Config {
                confidence_max: 1,
                .. Default::default()
            }).into(),
            ping: ping::Behaviour::default(),
            identify: identify::Behaviour::new(identify::Config::new(
                "/RelayDemo/0.0.1".to_string(),
                key.public(),
            )),
        })
        .expect("swarm with behaviour")
        .with_swarm_config(|c| {
            c.with_idle_connection_timeout(std::time::Duration::from_secs(u64::MAX))
        })
        .build();

    let listen_addr = Multiaddr::from(Ipv4Addr::UNSPECIFIED).with(Protocol::Tcp(opt.listen_port));
    swarm
        .listen_on(listen_addr)
        .expect("swarm listen on tcp normal");

    if let Some(port) = opt.dcutr_port {
        let listen_addr = Multiaddr::from(Ipv4Addr::UNSPECIFIED)
            .with(Protocol::Tcp(port))
            .with(Protocol::P2pWebRtcDirect);
        swarm
            .listen_on(listen_addr)
            .expect("swarm listen on tcp for dcutr");
    }

    // Wait to listen on all interfaces.
    block_on(async {
        let mut delay = futures_timer::Delay::new(std::time::Duration::from_secs(1)).fuse();
        loop {
            futures::select! {
                event = swarm.next() => {
                    match event.unwrap() {
                        SwarmEvent::NewListenAddr { address, .. } => {
                            tracing::info!(%address, "Listening on address");
                        }
                        event => panic!("{event:?}"),
                    }
                }
                _ = delay => {
                    // Likely listening on all interfaces now, thus continuing by breaking the loop.
                    break;
                }
            }
        }
    });

    for dest in opt.connect.iter().cloned() {
        if let Err(e) = swarm.dial(dest) {
            warn!("connect: {e:?}");
        }
    }

    block_on(async {
        info!("Swarm Loop");

        let mut connections: HashMap<PeerId, HashMap<ConnectionId, ConnectedPoint>>  = HashMap::new();
        let mut relayed_connections: HashMap<PeerId, HashSet<ConnectionId>> = HashMap::new();

        loop {
            match swarm.next().await.expect("swarm stream") {
                SwarmEvent::Behaviour(BehaviourEvent::Identify(evt)) => {
                    match evt {
                        identify::Event::Received { peer_id, info } => {
                            let _span = warn_span!("identify", ?peer_id).entered();
                            info!(?info, "received");

                            let is_relay_server = info.protocols.contains(&relay::HOP_PROTOCOL_NAME);
                            if is_relay_server {
                                info!("relay candidate");
                            }

                            if is_relay_server && opt.listen_relayed {
                                if let Some(addr) = connections.get(&peer_id).and_then(|c| c.values().find_map(|point| match point {
                                    ConnectedPoint::Dialer { address, role_override: Endpoint::Dialer } => Some(address.clone()),
                                    _ => None
                                })) {
                                    let listen_addr = addr.with(Protocol::P2pCircuit);
                                    let _inner_span = warn_span!("relayed", ?listen_addr).entered();
                                    match swarm.listen_on(listen_addr) {
                                        Ok(_) => info!("listened"),
                                        Err(e) => warn!(err=?e, "failed"),
                                    }
                                }
                            }

                            if let Some(peer_addr) = opt.peer.as_ref() {
                                let _peer_span = warn_span!("peer", ?peer_addr).entered();
                                let pre_connected = opt.connect.iter().any(|addr| addr.iter().any(|p| p == Protocol::P2p(peer_id)));
                                if pre_connected {
                                    match swarm.dial(peer_addr.clone()) {
                                        Ok(_) => info!("dialed"),
                                        Err(e) => warn!(err=?e, "dial failure"),
                                    };
                                }
                            }
                        },

                        _other => {},
                    }
                }

                SwarmEvent::Behaviour(BehaviourEvent::Autonat(evt)) => {
                    info!(?evt, "autonat");
                }

                SwarmEvent::Behaviour(BehaviourEvent::Dcutr(evt)) => {
                    info!(?evt, "DCUTR");
                    if let Some(conns) = relayed_connections.remove(&evt.remote_peer_id) {
                        for conn in conns {
                            let closed = swarm.close_connection(conn);
                            info!(?conn, ?closed, "close relayed connection");
                        }
                    }
                }

                SwarmEvent::Behaviour(BehaviourEvent::Kad(evt)) => {
                    info!(?evt, "kademlia");
                    if let kad::Event::RoutingUpdated { peer, .. } = evt {
                        let _kad_span = warn_span!("kad", ?peer);
                        if let Some(put) = opt.kad_put.as_ref() {
                            let mut splitted = put.splitn(2, ':');
                            let k = splitted.next().unwrap_or("").trim();
                            let v = splitted.next().unwrap_or("").trim();

                            if !k.is_empty() && !v.is_empty() {
                                let _put_span = warn_span!("put", k, v).entered();
                                if let Some(kad) = swarm.behaviour_mut().kad.as_mut() {
                                    let query_id =  kad.inner_mut().put_record_to(kad::Record{key: kad::RecordKey::new(&k), value: v.as_bytes().to_vec(), publisher: None, expires: None}, [peer].into_iter(), kad::Quorum::One);
                                    info!(?query_id, "query");
                                }
                            }
                        }

                        if let Some(key) = opt.kad_get.as_ref() {
                            let k = key.trim();
                            if !k.is_empty() {
                                let _get_span = warn_span!("kad get", k).entered();
                                if let Some(kad) = swarm.behaviour_mut().kad.as_mut() {
                                    let query_id =  kad.inner_mut().get_record(kad::RecordKey::new(&k));
                                    info!(?query_id, "get record");
                                }
                            }
                        }
                    }
                }

                SwarmEvent::ConnectionEstablished { peer_id, connection_id, endpoint, .. } => {
                    info!(?peer_id, ?connection_id, ?endpoint, "connection established");
                    if endpoint.is_relayed() {
                        relayed_connections.entry(peer_id).or_default().insert(connection_id);
                    }

                    connections.entry(peer_id).or_default().insert(connection_id, endpoint);
                }

                SwarmEvent::ConnectionClosed { peer_id, connection_id, endpoint, .. } => {
                    info!(?peer_id, ?connection_id, "connection closed");
                    if endpoint.is_relayed() {
                        relayed_connections.entry(peer_id).and_modify(|set| { set.remove(&connection_id); });
                    }

                    let entry = connections.entry(peer_id);
                    let mut is_empty = false;
                    entry.and_modify(|c| { c.remove(&connection_id); is_empty = c.is_empty(); });
                    if is_empty {
                        connections.remove(&peer_id);
                    }
                }

                event => {
                    debug!(?event, "OTHER EVENT<{}>", type_name_of_val(&event));
                }
            }
        }
    });
}
