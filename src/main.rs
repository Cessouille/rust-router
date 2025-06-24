use pnet::datalink;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct HelloMsg {
    router_id: String,
    networks: Vec<String>,
}

fn main() {
    let port = 9999;
    let interfaces = datalink::interfaces();

    // Gather all local networks
    let mut local_networks = Vec::new();
    for iface in &interfaces {
        for ip in &iface.ips {
            if let pnet::ipnetwork::IpNetwork::V4(ipv4) = ip {
                local_networks.push(ipv4.to_string());
            }
        }
    }

    let hello = HelloMsg {
        router_id: hostname::get().unwrap().to_string_lossy().to_string(),
        networks: local_networks.clone(),
    };
    let hello_bytes = serde_json::to_vec(&hello).unwrap();

    // Create a socket for listening (reuse for all rounds)
    let listen_socket = UdpSocket::bind(("0.0.0.0", port)).expect("Failed to bind listen socket");
    listen_socket.set_broadcast(true).unwrap();
    listen_socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    loop {
        // Broadcast hello on each interface
        for iface in &interfaces {
            for ip in &iface.ips {
                if let pnet::ipnetwork::IpNetwork::V4(ipv4) = ip {
                    let local_ip = ipv4.ip();
                    let broadcast_ip = ipv4.broadcast();
                    let ip_str = ipv4.to_string();
                    // Skip loopback and NAT/host-only interfaces
                    if ip_str.starts_with("127.") || ip_str.starts_with("10.0.2.") {
                        continue;
                    }
                    let socket = UdpSocket::bind((local_ip, 0)).expect("Failed to bind socket");
                    socket.set_broadcast(true).unwrap();
                    let dest = SocketAddrV4::new(broadcast_ip, port);
                    socket.send_to(&hello_bytes, dest).ok();
                    println!("Sent hello from {} to {}", local_ip, broadcast_ip);
                }
            }
        }

        // Listen for hello packets for a few seconds
        let start = Instant::now();
        let mut topology: HashMap<Ipv4Addr, HelloMsg> = HashMap::new();
        let mut buf = [0u8; 512];
        while start.elapsed().as_secs() < 2 {
            if let Ok((size, src)) = listen_socket.recv_from(&mut buf) {
                if let Ok(msg) = serde_json::from_slice::<HelloMsg>(&buf[..size]) {
                    if let std::net::SocketAddr::V4(src_v4) = src {
                        topology.insert(*src_v4.ip(), msg);
                    }
                }
            }
        }
        println!("Discovered topology: {:#?}", topology);

        // Add a route for each discovered network (except our own)
        for (neighbor_ip, msg) in &topology {
            // Ignore HelloMsg from ourselves (by router_id)
            if msg.router_id == hello.router_id {
                continue;
            }
            // Ignore HelloMsg that only advertises networks we already have (covers own IPs on all interfaces)
            if msg.networks.iter().all(|n| local_networks.contains(n)) {
                continue;
            }
            for net in &msg.networks {
                // Ignore our own networks, loopback, and NAT/host-only
                if local_networks.contains(net)
                    || net.starts_with("127.")
                    || net.starts_with("10.0.2.")
                {
                    continue;
                }
                println!("Adding route: ip route add {} via {}", net, neighbor_ip);
                // Uncomment to actually add the route:
                // let _ = std::process::Command::new("ip")
                //     .args(&["route", "add", net, "via", &neighbor_ip.to_string()])
                //     .status();
            }
        }

        // Wait before next round
        thread::sleep(Duration::from_secs(5));
    }
}
