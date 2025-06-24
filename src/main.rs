use pnet::datalink;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct HelloMsg {
    router_id: String,
    networks: Vec<(String, u32)>, // (network, hop_count)
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

    // Track: network -> (hop_count, via_neighbor, last_seen)
    let mut known_networks: HashMap<String, (u32, Ipv4Addr, Instant)> = local_networks
        .iter()
        .map(|n| (n.clone(), (0, Ipv4Addr::UNSPECIFIED, Instant::now())))
        .collect();

    let listen_socket = UdpSocket::bind(("0.0.0.0", port)).expect("Failed to bind listen socket");
    listen_socket.set_broadcast(true).unwrap();
    listen_socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    loop {
        // Build HelloMsg with all known networks and their hop counts
        let hello = HelloMsg {
            router_id: hostname::get().unwrap().to_string_lossy().to_string(),
            networks: known_networks
                .iter()
                .map(|(n, (h, _, _))| (n.clone(), *h))
                .collect(),
        };
        let _hello_bytes = serde_json::to_vec(&hello).unwrap();

        // Broadcast hello on each interface
        for iface in &interfaces {
            for ip in &iface.ips {
                if let pnet::ipnetwork::IpNetwork::V4(ipv4) = ip {
                    let ip_str = format!("{}/{}", ipv4.network(), ipv4.prefix());
                    if ip_str.starts_with("127.") || ip_str.starts_with("10.0.2.") {
                        continue;
                    }
                    let local_ip = ipv4.ip();
                    let broadcast_ip = ipv4.broadcast();

                    // Split horizon: don't advertise routes learned from this neighbor
                    let hello = HelloMsg {
                        router_id: hostname::get().unwrap().to_string_lossy().to_string(),
                        networks: known_networks
                            .iter()
                            .filter(|(_net, (_hops, via, _))| {
                                *via != local_ip && *via != Ipv4Addr::UNSPECIFIED
                            })
                            .map(|(n, (h, _, _))| (n.clone(), *h))
                            .collect(),
                    };
                    let hello_bytes = serde_json::to_vec(&hello).unwrap();

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

        // Learn new networks from neighbors and update known_networks
        for (neighbor_ip, msg) in &topology {
            for (net, neighbor_hops) in &msg.networks {
                if net.starts_with("127.") || net.starts_with("10.0.2.") {
                    continue;
                }
                let new_hops = neighbor_hops + 1;
                let update = match known_networks.get(net) {
                    Some(&(existing_hops, _, _)) => new_hops < existing_hops,
                    None => true,
                };
                if update {
                    known_networks.insert(net.clone(), (new_hops, *neighbor_ip, Instant::now()));
                }
            }
        }

        // Remove expired routes (not seen for 15 seconds)
        let expire_duration = Duration::from_secs(15);
        known_networks.retain(|net, &mut (_hops, _via, last_seen)| {
            last_seen.elapsed() < expire_duration || local_networks.contains(net)
        });

        // Remove expired routes from system
        for (net, (_hops, _via, last_seen)) in &known_networks {
            if !local_networks.contains(net) && last_seen.elapsed() >= expire_duration {
                println!("Removing route: ip route del {}", net);
                let _ = std::process::Command::new("ip")
                    .args(&["route", "del", net])
                    .status();
            }
        }

        // Add or update a route for each discovered network (except our own)
        for (net, (hops, neighbor_ip, _last_seen)) in &known_networks {
            if *hops == 0
                || local_networks.contains(net)
                || net.starts_with("127.")
                || net.starts_with("10.0.2.")
            {
                continue;
            }
            // Only add route if net is a valid network address (not a host address)
            let parts: Vec<&str> = net.split('/').collect();
            if parts.len() == 2 {
                let addr = parts[0];
                let prefix = parts[1];
                if prefix == "24" {
                    let octets: Vec<&str> = addr.split('.').collect();
                    if octets.len() == 4 && octets[3] != "0" {
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

        // Wait before next round
        thread::sleep(Duration::from_secs(5));
    }
}
