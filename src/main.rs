use pnet::datalink;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct HelloMsg {
    router_id: String,
    networks: Vec<(String, u32)>,
}

fn main() {
    let neighbors = Arc::new(Mutex::new(HashMap::<Ipv4Addr, String>::new()));
    let running = Arc::new(AtomicBool::new(false));
    let mut handle: Option<std::thread::JoinHandle<()>> = None;

    loop {
        println!("\n==============================");
        println!("      Rust Router CLI");
        println!("==============================");
        println!(
            "Dynamic routing: {}",
            if running.load(Ordering::SeqCst) {
                "ENABLED"
            } else {
                "DISABLED"
            }
        );
        println!("1. Enable/Disable dynamic routing");
        println!("2. List last known neighbor routers");
        println!("3. Exit");
        print!("> Enter your choice: ");
        io::stdout().flush().unwrap();

        let mut choice = String::new();
        io::stdin().read_line(&mut choice).unwrap();
        match choice.trim() {
            "1" => {
                if running.load(Ordering::SeqCst) {
                    println!("\n[!] Disabling dynamic routing...");
                    running.store(false, Ordering::SeqCst);
                    if let Some(h) = handle.take() {
                        h.join().ok();
                    }
                } else {
                    println!("\n[+] Enabling dynamic routing...");
                    running.store(true, Ordering::SeqCst);
                    let running_clone = running.clone();
                    let neighbors_clone = neighbors.clone();
                    handle = Some(thread::spawn(move || {
                        run_dynamic_routing(running_clone, neighbors_clone);
                    }));
                }
            }
            "2" => {
                list_neighbors(&neighbors);
            }
            "3" => {
                println!("\nExiting. Goodbye!");
                running.store(false, Ordering::SeqCst);
                if let Some(h) = handle.take() {
                    h.join().ok();
                }
                break;
            }
            _ => println!("\n[!] Invalid choice! Please enter 1, 2, or 3."),
        }
    }
}

fn run_dynamic_routing(running: Arc<AtomicBool>, neighbors: Arc<Mutex<HashMap<Ipv4Addr, String>>>) {
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

    let mut known_networks: HashMap<String, (u32, Ipv4Addr, Instant)> = local_networks
        .iter()
        .map(|n| (n.clone(), (0, Ipv4Addr::UNSPECIFIED, Instant::now())))
        .collect();

    let listen_socket = UdpSocket::bind(("0.0.0.0", port)).expect("Failed to bind listen socket");
    listen_socket.set_broadcast(true).unwrap();
    listen_socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    while running.load(Ordering::SeqCst) {
        // Broadcast hello on each interface (split horizon)
        for iface in &interfaces {
            for ip in &iface.ips {
                if let pnet::ipnetwork::IpNetwork::V4(ipv4) = ip {
                    let ip_str = format!("{}/{}", ipv4.network(), ipv4.prefix());
                    if ip_str.starts_with("127.") || ip_str.starts_with("10.0.2.") {
                        continue;
                    }
                    let local_ip = ipv4.ip();
                    let broadcast_ip = ipv4.broadcast();

                    let hello = HelloMsg {
                        router_id: hostname::get().unwrap().to_string_lossy().to_string(),
                        networks: known_networks
                            .iter()
                            .filter(|(_net, (_hops, via, _))| *via != local_ip)
                            .map(|(n, (h, _, _))| (n.clone(), *h))
                            .collect(),
                    };
                    let hello_bytes = serde_json::to_vec(&hello).unwrap();

                    let socket = UdpSocket::bind((local_ip, 0)).expect("Failed to bind socket");
                    socket.set_broadcast(true).unwrap();
                    let dest = SocketAddrV4::new(broadcast_ip, port);
                    socket.send_to(&hello_bytes, dest).ok();
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
                        if msg.router_id != hostname::get().unwrap().to_string_lossy() {
                            topology.insert(*src_v4.ip(), msg.clone());
                        }
                    }
                }
            }
        }

        // Update last known neighbors
        let mut neigh = neighbors.lock().unwrap();
        neigh.clear();
        for (ip, msg) in &topology {
            neigh.insert(*ip, msg.router_id.clone());
        }

        // Learn new networks from neighbors and update known_networks
        for (neighbor_ip, msg) in &topology {
            for (net, neighbor_hops) in &msg.networks {
                if net.starts_with("127.") || net.starts_with("10.0.2.") {
                    continue;
                }
                let new_hops = neighbor_hops + 1;
                let update = match known_networks.get(net) {
                    Some(&(existing_hops, existing_via, _)) => {
                        if new_hops < existing_hops {
                            true
                        } else if new_hops == existing_hops {
                            // Tie-breaker: prefer lower IP address
                            *neighbor_ip < existing_via
                        } else {
                            false
                        }
                    }
                    None => true,
                };
                if update {
                    known_networks.insert(net.clone(), (new_hops, *neighbor_ip, Instant::now()));
                }
            }
        }

        // Remove expired routes from system
        let expire_duration = Duration::from_secs(15);
        for (net, (_hops, _via, last_seen)) in &known_networks {
            if last_seen.elapsed() >= expire_duration {
                let _ = std::process::Command::new("ip")
                    .args(&["route", "del", net])
                    .status();
            }
        }

        // Now actually remove expired learned routes from known_networks
        known_networks
            .retain(|_net, &mut (_hops, _via, last_seen)| last_seen.elapsed() < expire_duration);

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
            let _ = std::process::Command::new("ip")
                .args(&["route", "replace", net, "via", &neighbor_ip.to_string()])
                .status();
        }

        thread::sleep(Duration::from_secs(5));
    }
}

fn list_neighbors(neighbors: &Arc<Mutex<HashMap<Ipv4Addr, String>>>) {
    let neigh = neighbors.lock().unwrap();
    println!("\n------------------------------");
    if neigh.is_empty() {
        println!("No neighbors discovered yet.");
    } else {
        println!("Last known neighbor routers:");
        for (ip, name) in neigh.iter() {
            println!("  • {:<15}  {}", ip, name);
        }
    }
    println!("------------------------------");
}
