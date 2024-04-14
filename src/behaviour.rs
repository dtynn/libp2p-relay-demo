use libp2p::{
    autonat, identify, ping, relay,
    swarm::{behaviour::toggle::Toggle, NetworkBehaviour},
};

mod direct_client;

#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub relay: Toggle<relay::Behaviour>,
    pub relay_client: relay::client::Behaviour,
    pub dcutr: Toggle<direct_client::Behaviour>,
    pub autonat: autonat::Behaviour,
    pub ping: ping::Behaviour,
    pub identify: identify::Behaviour,
}
