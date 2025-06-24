use pnet::datalink;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

#[derive(Serialize, Deserialize, Debug)]
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

    // Listen for hello packets
    let listener = std::thread::spawn(move || {
        let listen_socket =
            UdpSocket::bind(("0.0.0.0", port)).expect("Failed to bind listen socket");
        listen_socket.set_broadcast(true).unwrap();
        let mut buf = [0u8; 512];
        let start = std::time::Instant::now();
        let mut topology: HashMap<Ipv4Addr, HelloMsg> = HashMap::new();
        while start.elapsed().as_secs() < 2 {
            if let Ok((size, src)) = listen_socket.recv_from(&mut buf) {
                if let Ok(msg) = serde_json::from_slice::<HelloMsg>(&buf[..size]) {
                    if let std::net::SocketAddr::V4(src_v4) = src {
                        topology.insert(*src_v4.ip(), msg);
                    }
                }
            }
        }
        topology
    });

    // Broadcast hello on each interface
    for iface in &interfaces {
        for ip in &iface.ips {
            if let pnet::ipnetwork::IpNetwork::V4(ipv4) = ip {
                let local_ip = ipv4.ip();
                let broadcast_ip = ipv4.broadcast();
                let socket = UdpSocket::bind((local_ip, 0)).expect("Failed to bind socket");
                socket.set_broadcast(true).unwrap();
                let dest = SocketAddrV4::new(broadcast_ip, port);
                socket.send_to(&hello_bytes, dest).ok();
                println!("Sent hello from {} to {}", local_ip, broadcast_ip);
            }
        }
    }

    // Wait for neighbor discovery to finish
    let topology = listener.join().unwrap();
    println!("Discovered topology: {:#?}", topology);

    // Add a route for each discovered network (except our own)
    for (neighbor_ip, msg) in &topology {
        for net in &msg.networks {
            if !local_networks.contains(net) {
                println!("Adding route: ip route add {} via {}", net, neighbor_ip);
                // Uncomment to actually add the route:
                // let _ = std::process::Command::new("ip")
                //     .args(&["route", "add", net, "via", &neighbor_ip.to_string()])
                //     .status();
            }
        }
    }
}
