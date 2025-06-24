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
                let ip_str = format!("{}/{}", ipv4.network(), ipv4.prefix());
                // Skip loopback and NAT/host-only interfaces
                if ip_str.starts_with("127.") || ip_str.starts_with("10.0.2.") {
                    continue;
                }
                local_networks.push(ip_str);
            }
        }
    }

    let hello = HelloMsg {
        router_id: hostname::get().unwrap().to_string_lossy().to_string(),
        networks: local_networks.clone(),
    };
    let _hello_bytes = serde_json::to_vec(&hello).unwrap();

    // Create a socket for listening (reuse for all rounds)
    let listen_socket = UdpSocket::bind(("0.0.0.0", port)).expect("Failed to bind listen socket");
    listen_socket.set_broadcast(true).unwrap();
    listen_socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let mut known_networks = local_networks.clone();
    // Track: network -> (via_neighbor, last_seen)
    let mut route_table: HashMap<String, (Ipv4Addr, Instant)> = HashMap::new();

    loop {
        // Broadcast hello on each interface, advertising all known networks
        let hello = HelloMsg {
            router_id: hostname::get().unwrap().to_string_lossy().to_string(),
            networks: known_networks.clone(),
        };
        let hello_bytes = serde_json::to_vec(&hello).unwrap();

        for iface in &interfaces {
            for ip in &iface.ips {
                if let pnet::ipnetwork::IpNetwork::V4(ipv4) = ip {
                    let ip_str = format!("{}/{}", ipv4.network(), ipv4.prefix());
                    // Skip loopback and NAT/host-only interfaces
                    if ip_str.starts_with("127.") || ip_str.starts_with("10.0.2.") {
                        continue;
                    }
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

        // Listen for hello packets for a few seconds
        let start = Instant::now();
        let mut topology: HashMap<Ipv4Addr, HelloMsg> = HashMap::new();
        let mut buf = [0u8; 512];
        while start.elapsed().as_secs() < 2 {
            if let Ok((size, src)) = listen_socket.recv_from(&mut buf) {
                if let Ok(msg) = serde_json::from_slice::<HelloMsg>(&buf[..size]) {
                    if let std::net::SocketAddr::V4(src_v4) = src {
                        // Ignore messages from ourselves
                        if msg.router_id != hello.router_id {
                            topology.insert(*src_v4.ip(), msg);
                        }
                    }
                }
            }
        }
        println!("Discovered topology: {:#?}", topology);

        // Learn new networks from neighbors and update route_table
        for (neighbor_ip, msg) in &topology {
            for net in &msg.networks {
                if !net.starts_with("127.") && !net.starts_with("10.0.2.") {
                    // Update route_table with the latest info
                    route_table.insert(net.clone(), (*neighbor_ip, Instant::now()));
                    if !known_networks.contains(net) {
                        known_networks.push(net.clone());
                    }
                }
            }
        }

        // Remove expired routes (not seen for 15 seconds)
        let expire_duration = Duration::from_secs(15);
        route_table.retain(|_net, (_via, last_seen)| last_seen.elapsed() < expire_duration);
        known_networks.retain(|net| route_table.contains_key(net) || local_networks.contains(net));

        // Remove expired routes from system
        for net in known_networks
            .iter()
            .filter(|n| !route_table.contains_key(*n) && !local_networks.contains(*n))
        {
            println!("Removing route: ip route del {}", net);
            let _ = std::process::Command::new("ip")
                .args(&["route", "del", net])
                .status();
        }

        // Add or update a route for each discovered network (except our own)
        for (net, (neighbor_ip, _last_seen)) in &route_table {
            if !local_networks.contains(net)
                && !net.starts_with("127.")
                && !net.starts_with("10.0.2.")
            {
                // Only add route if net is a valid network address (not a host address)
                let parts: Vec<&str> = net.split('/').collect();
                if parts.len() == 2 {
                    let addr = parts[0];
                    let prefix = parts[1];
                    if prefix == "24" {
                        let octets: Vec<&str> = addr.split('.').collect();
                        if octets.len() == 4 && octets[3] != "0" {
                            // Not a network address for /24, skip
                            continue;
                        }
                    }
                }
                println!(
                    "Adding/replacing route: ip route replace {} via {}",
                    net, neighbor_ip
                );
                let _ = std::process::Command::new("ip")
                    .args(&["route", "replace", net, "via", &neighbor_ip.to_string()])
                    .status();
            }
        }

        // Wait before next round
        thread::sleep(Duration::from_secs(5));
    }
}
