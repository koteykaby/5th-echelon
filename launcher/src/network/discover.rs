use std::net::IpAddr;
use std::time::Duration;

use quazal::prudp::packet::PacketFlag;
use quazal::prudp::packet::PacketType;
use quazal::prudp::packet::QPacket;
use quazal::prudp::packet::StreamType;
use quazal::prudp::packet::VPort;
use tokio::net::UdpSocket;
use tokio::task::JoinSet;
use tracing::info;

use super::Error;

pub async fn try_locate_server(
    adapter: Option<(&str, IpAddr)>,
    adapters: &[(&str, IpAddr)],
) -> Result<IpAddr, Error> {
    tokio::time::timeout(
        Duration::from_secs(5),
        try_locate_server_inner(adapter, adapters),
    )
    .await
    .map_err(|_| Error::TimedOut)?
}

async fn try_locate_server_inner(
    adapter: Option<(&str, IpAddr)>,
    adapters: &[(&str, IpAddr)],
) -> Result<IpAddr, Error> {
    if let Some((name, ip)) = adapter {
        info!("Trying to locate server through adapter {name} ({ip})");
        try_locate_server_from_ip(ip).await
    } else {
        info!("Trying to locate server on all interfaces");
        let mut set = JoinSet::new();

        for (name, ip) in adapters {
            info!("Trying to locate server through adapter {name} ({ip})");
            let ip = *ip;
            set.spawn(async move { try_locate_server_from_ip(ip).await });
        }
        let mut res = Err(Error::ConnectionFailed);
        while let Some(Ok(r)) = set.join_next().await {
            if r.is_ok() {
                return r;
            }
            res = r;
        }
        res
    }
}

async fn try_locate_server_from_ip(ip: IpAddr) -> Result<IpAddr, Error> {
    let ctx = quazal::Context::splinter_cell_blacklist();
    let socket = UdpSocket::bind(format!("{}:0", ip)).await?;
    socket.set_broadcast(true)?;
    let syn_pkt = QPacket {
        source: VPort {
            port: 15,
            stream_type: StreamType::RVSec,
        },
        destination: VPort {
            port: 1,
            stream_type: StreamType::RVSec,
        },
        packet_type: PacketType::Syn,
        flags: PacketFlag::NeedAck.into(),
        conn_signature: Some(0),
        session_id: rand::random(),
        ..Default::default()
    };
    let buf = syn_pkt.to_bytes(&ctx);
    socket.send_to(&buf, "255.255.255.255:21126").await?;
    let mut buf = vec![0u8; 4096];

    let (n, peer) = socket.recv_from(&mut buf).await?;
    let (syn_ack_pkt, _size) =
        QPacket::from_bytes(&ctx, &buf[..n]).map_err(|e| std::io::Error::other(e.to_string()))?;

    if syn_ack_pkt.packet_type != PacketType::Syn || !syn_ack_pkt.flags.contains(PacketFlag::Ack) {
        return Err(Error::IO(std::io::Error::other("invalid syn ack")));
    }
    Ok(peer.ip())
}
