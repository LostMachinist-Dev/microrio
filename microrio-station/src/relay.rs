//! Telemetry relayed from the station that owns a node to the ones watching.
//!
//! A board answers to one station. Any other station used to ask the board
//! directly and the board answered — which is fine until you measure it. A
//! W5100 resolves the destination before every single packet, so a watcher on
//! a laptop over WiFi, where a sleeping radio answers that in its own time,
//! cost the board 377 ms per frame. The machine being driven stuttered
//! because somebody opened a second window to look at it.
//!
//! So the board never sends to a watcher. The station driving it forwards
//! what it receives, and a computer has an address cache, a real network
//! stack, and nothing to do between frames.
//!
//! The relay is a broadcast on one UDP port, carrying frames exactly as they
//! came off the wire: the same header, the same body, so a watcher decodes
//! them with the same code as the station that owns them. The schema goes out
//! with them every couple of seconds, because a watcher that never speaks to
//! the board cannot ask for it.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

use microrio_proto::schema::Schema;
use microrio_proto::{Header, MsgType};
use tokio::net::UdpSocket;

/// Where relayed frames go: the worst a stray listener can do is watch.
pub use crate::config::DEFAULT_RELAY_PORT as DEFAULT_PORT;

/// How often a watcher is reminded what the node's channels are. A watcher
/// that starts late, or one whose node was reflashed, is drawing within this.
const SCHEMA_EVERY: Duration = Duration::from_secs(2);

/// Sends what this station receives, for the stations that cannot ask.
pub struct Relay {
    sock: UdpSocket,
    /// Where a copy goes. Two places, for two kinds of watcher.
    to: Vec<SocketAddr>,
    last_schema: Instant,
}

impl Relay {
    /// Bind a sender for one node. Fails with a reason rather than stopping
    /// a station starting: relaying is a courtesy, not the job.
    ///
    /// `node` is the board being relayed, and it decides where the copies go:
    /// the broadcast address of its own network, which is exactly the set of
    /// machines that could be watching it.
    pub async fn bind(port: u16, node: Ipv4Addr) -> Result<Self, String> {
        let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .await
            .map_err(|e| format!("relay socket: {e}"))?;
        sock.set_broadcast(true)
            .map_err(|e| format!("relay broadcast: {e}"))?;
        Ok(Self {
            sock,
            to: destinations(port, node),
            // Far enough in the past that the first frame carries a schema.
            last_schema: Instant::now() - SCHEMA_EVERY,
        })
    }

    /// Forward one telemetry frame exactly as it arrived.
    pub async fn frame(&self, raw: &[u8]) {
        for to in &self.to {
            let _ = self.sock.send_to(raw, to).await;
        }
    }

    /// Send the node's schema, if it is time. Cheap to call every frame.
    pub async fn schema(&mut self, node_id: u16, schema: &Schema) {
        if self.last_schema.elapsed() < SCHEMA_EVERY {
            return;
        }
        self.last_schema = Instant::now();
        if let Ok(chunks) = schema.encode_chunks(node_id, 0) {
            for c in chunks {
                for to in &self.to {
                    let _ = self.sock.send_to(&c, to).await;
                }
            }
        }
    }
}

/// Where a copy of each frame goes.
///
/// Two places, for two kinds of watcher: this computer, because a broadcast
/// is not delivered back to the machine that sent it and a second window on
/// one screen still needs a copy; and the board's own network, which is
/// exactly the set of machines that could be watching it.
fn destinations(port: u16, node: Ipv4Addr) -> Vec<SocketAddr> {
    let mut out = vec![SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))];
    // A board addressed on loopback is only ever reachable from here, and
    // broadcasting on loopback would just deliver a second copy.
    if !node.is_loopback() {
        out.push(SocketAddr::V4(SocketAddrV4::new(
            subnet_broadcast(node),
            port,
        )));
    }
    out
}

/// The broadcast address of the network a board is on.
///
/// Assumes a /24, which is what a hand-numbered bench network is. Getting it
/// wrong costs a watcher on another machine its picture, never anything on
/// the board: nothing relayed is a command.
fn subnet_broadcast(node: Ipv4Addr) -> Ipv4Addr {
    let o = node.octets();
    Ipv4Addr::new(o[0], o[1], o[2], 255)
}

/// What a watcher pulled off the relay.
pub enum Relayed {
    Schema(u16, Vec<u8>),
    Telemetry(u16, u32, Vec<u8>),
}

/// Listens for relayed frames, for the nodes this station is watching.
pub struct Listener {
    sock: UdpSocket,
    buf: Vec<u8>,
}

impl Listener {
    /// Bind the relay port for listening.
    ///
    /// Reuse is on, so two stations watching from the same computer both
    /// hear it — the usual case being a driving station and a second window
    /// on another screen.
    pub fn bind(port: u16) -> Result<Self, String> {
        let sock = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )
        .map_err(|e| format!("relay listener: {e}"))?;
        sock.set_reuse_address(true)
            .map_err(|e| format!("relay listener reuse: {e}"))?;
        sock.set_nonblocking(true)
            .map_err(|e| format!("relay listener nonblocking: {e}"))?;
        sock.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())
            .map_err(|e| format!("relay listener bind {port}: {e}"))?;
        let sock = UdpSocket::from_std(sock.into()).map_err(|e| format!("relay listener: {e}"))?;
        Ok(Self {
            sock,
            buf: vec![0u8; 2048],
        })
    }

    /// The next relayed frame. Anything malformed is skipped rather than
    /// reported: this is a broadcast port and whatever else is on it is not
    /// this station's business.
    pub async fn next(&mut self) -> Relayed {
        loop {
            let Ok((n, _)) = self.sock.recv_from(&mut self.buf).await else {
                continue;
            };
            let raw = &self.buf[..n];
            let Ok((hdr, body)) = Header::decode(raw) else {
                continue;
            };
            match hdr.msg {
                MsgType::Schema => return Relayed::Schema(hdr.node_id, body.to_vec()),
                MsgType::Telem => {
                    return Relayed::Telemetry(hdr.node_id, hdr.schema_hash, body.to_vec())
                }
                _ => continue,
            }
        }
    }
}
