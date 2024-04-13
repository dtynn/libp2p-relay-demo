use libp2p::{
    autonat, dcutr, identify, ping, relay,
    swarm::{behaviour::toggle::Toggle, NetworkBehaviour},
};

#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub relay: Toggle<relay::Behaviour>,
    pub relay_client: relay::client::Behaviour,
    pub dcutr: Toggle<dcutr::Behaviour>,
    pub autonat: autonat::Behaviour,
    pub ping: ping::Behaviour,
    pub identify: identify::Behaviour,
}
