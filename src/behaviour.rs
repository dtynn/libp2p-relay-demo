use libp2p::{
    identify, ping, relay,
    swarm::{behaviour::toggle::Toggle, NetworkBehaviour},
};

mod autonat;
mod direct_client;
mod kad;

#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub kad: Toggle<kad::Behaviour>,
    pub relay: Toggle<relay::Behaviour>,
    pub relay_client: relay::client::Behaviour,
    pub dcutr: Toggle<direct_client::Behaviour>,
    pub autonat: autonat::Behaviour,
    pub ping: ping::Behaviour,
    pub identify: identify::Behaviour,
}
