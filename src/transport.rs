use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use tracing::info;

use libp2p::{
    core::{
        address_translation,
        transport::{ListenerId, TransportEvent},
    },
    multiaddr::Protocol,
    tcp::{tokio::Transport as TokioTcpTransport, Config},
    Multiaddr, Transport, TransportError,
};

pub fn is_holepunch_direct_addr(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| p == Protocol::P2pWebRtcDirect)
}

fn direct_addr_2_normal(addr: Multiaddr) -> Multiaddr {
    addr.into_iter()
        .filter(|p| !matches!(p, Protocol::P2pWebRtcDirect))
        .collect()
}

pub struct HolePunchTransport {
    listened: HashMap<ListenerId, Multiaddr>,
    inner: TokioTcpTransport,
}

impl HolePunchTransport {
    pub fn new(cfg: Config) -> Self {
        HolePunchTransport {
            listened: Default::default(),
            inner: TokioTcpTransport::new(cfg.port_reuse(true)),
        }
    }
}

impl Transport for HolePunchTransport {
    type Output = <TokioTcpTransport as Transport>::Output;
    type Error = <TokioTcpTransport as Transport>::Error;
    type ListenerUpgrade = <TokioTcpTransport as Transport>::ListenerUpgrade;
    type Dial = <TokioTcpTransport as Transport>::Dial;

    fn listen_on(
        &mut self,
        id: ListenerId,
        addr: Multiaddr,
    ) -> Result<(), TransportError<Self::Error>> {
        if is_holepunch_direct_addr(&addr) {
            info!(?id, ?addr, "listen on");
            let normal = direct_addr_2_normal(addr);
            self.inner.listen_on(id, normal.clone()).inspect(|_| {
                self.listened.insert(id, normal);
            })
        } else {
            Err(TransportError::MultiaddrNotSupported(addr))
        }
    }

    fn remove_listener(&mut self, id: ListenerId) -> bool {
        self.listened.remove(&id);
        self.inner.remove_listener(id)
    }

    fn dial(&mut self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        if is_holepunch_direct_addr(&addr) {
            info!(?addr, "dial");
            self.inner.dial(direct_addr_2_normal(addr))
        } else {
            Err(TransportError::MultiaddrNotSupported(addr))
        }
    }

    fn dial_as_listener(
        &mut self,
        addr: Multiaddr,
    ) -> Result<Self::Dial, TransportError<Self::Error>> {
        if is_holepunch_direct_addr(&addr) {
            info!(?addr, "dial as listener");
            self.inner.dial_as_listener(direct_addr_2_normal(addr))
        } else {
            Err(TransportError::MultiaddrNotSupported(addr))
        }
    }

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        Pin::new(&mut self.inner).poll(cx)
    }

    fn address_translation(&self, listen: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        let port = listen.iter().skip(1).next();

        self.listened
            .values()
            .any(|addr| {
                let addr_port = addr.iter().skip(1).next();
                addr_port.is_some() && addr_port == port
            })
            .then(|| address_translation(listen, observed))
            .flatten()
            .map(|addr| addr.with(Protocol::P2pWebRtcDirect))
            .inspect(|addr| info!(?addr, "translated"))
    }
}
