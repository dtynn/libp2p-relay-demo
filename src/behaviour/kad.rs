use std::task::{Context, Poll};

use either::Either;
use libp2p::{
    core::Endpoint,
    kad::{self, store::MemoryStore},
    multiaddr::Protocol,
    swarm::{
        dummy, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler,
        THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};

fn is_relayed(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| p == Protocol::P2pCircuit)
}

pub struct Behaviour {
    inner: kad::Behaviour<MemoryStore>,
}

impl Behaviour {
    pub fn inner_mut(&mut self) -> &mut kad::Behaviour<MemoryStore> {
        &mut self.inner
    }
}

impl From<kad::Behaviour<MemoryStore>> for Behaviour {
    fn from(value: kad::Behaviour<MemoryStore>) -> Self {
        Behaviour { inner: value }
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = Either<
        <kad::Behaviour<MemoryStore> as NetworkBehaviour>::ConnectionHandler,
        dummy::ConnectionHandler,
    >;
    type ToSwarm = <kad::Behaviour<MemoryStore> as NetworkBehaviour>::ToSwarm;

    fn handle_pending_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<(), ConnectionDenied> {
        self.inner
            .handle_pending_inbound_connection(connection_id, local_addr, remote_addr)
    }

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        if is_relayed(local_addr) {
            Ok(Either::Right(dummy::ConnectionHandler))
        } else {
            self.inner
                .handle_established_inbound_connection(connection_id, peer, local_addr, remote_addr)
                .map(Either::Left)
        }
    }

    fn handle_pending_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        maybe_peer: Option<PeerId>,
        addresses: &[Multiaddr],
        effective_role: Endpoint,
    ) -> Result<Vec<Multiaddr>, ConnectionDenied> {
        self.inner.handle_pending_outbound_connection(
            connection_id,
            maybe_peer,
            addresses,
            effective_role,
        )
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        addr: &Multiaddr,
        role_override: Endpoint,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        if is_relayed(addr) {
            Ok(Either::Right(dummy::ConnectionHandler))
        } else {
            self.inner
                .handle_established_outbound_connection(connection_id, peer, addr, role_override)
                .map(Either::Left)
        }
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        self.inner.on_swarm_event(event)
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        if let Either::Left(event) = event {
            self.inner
                .on_connection_handler_event(peer_id, connection_id, event)
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        self.inner
            .poll(cx)
            .map(|to_swarm| to_swarm.map_in(Either::Left))
    }
}
