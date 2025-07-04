use chrono::Local;
use pnet::datalink;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::OpenOptions;
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
    networks: Vec<(String, u32)>, // (network, hop_count)
}

fn main() {
    // Shared state for neighbor list and dynamic routing status
    let neighbors = Arc::new(Mutex::new(HashMap::<Ipv4Addr, String>::new()));
    let running = Arc::new(AtomicBool::new(false));
    let mut handle: Option<std::thread::JoinHandle<()>> = None;

    loop {
        // CLI menu
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
                // Toggle dynamic routing thread
                if running.load(Ordering::SeqCst) {
                    println!("\n[!] Disabling dynamic routing...");
                    running.store(false, Ordering::SeqCst);
                    if let Some(h) = handle.take() {
                        h.join().ok(); // Wait for thread to finish
                    }
                } else {
                    println!("\n[+] Enabling dynamic routing...");
                    running.store(true, Ordering::SeqCst);
                    let running_clone = running.clone();
                    let neighbors_clone = neighbors.clone();
                    // Spawn routing logic in a background thread
                    handle = Some(thread::spawn(move || {
                        run_dynamic_routing(running_clone, neighbors_clone);
                    }));
                }
            }
            "2" => {
                // Print last known neighbors
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

/// Main dynamic routing logic, runs in a background thread
fn run_dynamic_routing(running: Arc<AtomicBool>, neighbors: Arc<Mutex<HashMap<Ipv4Addr, String>>>) {
    // Open log file for performance logs
    let mut log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("router_perf.log")
        .expect("Unable to open log file");

    let port = 9999;
    let interfaces = datalink::interfaces();

    // Gather all local networks (excluding loopback and NAT/host-only)
    let mut local_networks = Vec::new();
    for iface in &interfaces {
        for ip in &iface.ips {
            if let pnet::ipnetwork::IpNetwork::V4(ipv4) = ip {
                let ip_str = format!("{}/{}", ipv4.network(), ipv4.prefix());
                if ip_str.starts_with("127.") || ip_str.starts_with("10.0.2.") {
                    continue;
                }
                local_networks.push(ip_str);
            }
        }
    }

    // known_networks: network -> (hop_count, via_neighbor, last_seen)
    let mut known_networks: HashMap<String, (u32, Ipv4Addr, Instant)> = local_networks
        .iter()
        .map(|n| (n.clone(), (0, Ipv4Addr::UNSPECIFIED, Instant::now())))
        .collect();

    // UDP socket for sending/receiving hello messages
    let listen_socket = UdpSocket::bind(("0.0.0.0", port)).expect("Failed to bind listen socket");
    listen_socket.set_broadcast(true).unwrap();
    listen_socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    while running.load(Ordering::SeqCst) {
        writeln!(
            log_file,
            "\n========================== CYCLE =========================="
        )
        .unwrap();

        let loop_start = Instant::now();

        // --- Send hello messages (split horizon) ---
        let send_start = Instant::now();
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
                            .filter(|(_net, (_hops, via, _))| *via != local_ip)
                            .map(|(n, (h, _, _))| (n.clone(), *h))
                            .collect(),
                    };
                    let hello_bytes = serde_json::to_vec(&hello).unwrap();

                    // Bind to local IP and send broadcast
                    let socket = UdpSocket::bind((local_ip, 0)).expect("Failed to bind socket");
                    socket.set_broadcast(true).unwrap();
                    let dest = SocketAddrV4::new(broadcast_ip, port);
                    socket.send_to(&hello_bytes, dest).ok();
                }
            }
        }
        let send_duration = send_start.elapsed();
        writeln!(
            log_file,
            "[{}] Time to send hellos: {:.2?}",
            Local::now().format("%d/%m/%Y %H:%M:%S"),
            send_duration
        )
        .unwrap();

        // --- Listen for hello packets from neighbors ---
        let recv_start = Instant::now();
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
        let recv_duration = recv_start.elapsed();
        writeln!(
            log_file,
            "[{}] Time spent receiving hellos: {:.2?}",
            Local::now().format("%d/%m/%Y %H:%M:%S"),
            recv_duration
        )
        .unwrap();
        writeln!(
            log_file,
            "[{}] Number of hellos received: {}",
            Local::now().format("%d/%m/%Y %H:%M:%S"),
            topology.len()
        )
        .unwrap();

        // --- Update last known neighbors (shared with CLI) ---
        let mut neigh = neighbors.lock().unwrap();
        neigh.clear();
        for (ip, msg) in &topology {
            neigh.insert(*ip, msg.router_id.clone());
        }

        // --- Learn new networks from neighbors and update known_networks ---
        for (neighbor_ip, msg) in &topology {
            for (net, neighbor_hops) in &msg.networks {
                if net.starts_with("127.") || net.starts_with("10.0.2.") {
                    continue;
                }
                let new_hops = neighbor_hops + 1;
                // Only update if new path is better (lower hop count), or tie-breaker (lower IP)
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

        // --- Remove expired routes from known_networks (not seen for 15 seconds) ---
        let expire_duration = Duration::from_secs(15);
        known_networks.retain(|net, &mut (_hops, _via, last_seen)| {
            last_seen.elapsed() < expire_duration || local_networks.contains(net)
        });

        // --- Remove expired routes from system ---
        for (net, (_hops, _via, last_seen)) in &known_networks {
            if !local_networks.contains(net) && last_seen.elapsed() >= expire_duration {
                let _ = std::process::Command::new("ip")
                    .args(&["route", "del", net])
                    .status();
            }
        }

        // --- Add or update a route for each discovered network (except our own) ---
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

        let loop_duration = loop_start.elapsed();
        writeln!(
            log_file,
            "[{}] Full protocol loop duration: {:.2?}",
            Local::now().format("%d/%m/%Y %H:%M:%S"),
            loop_duration
        )
        .unwrap();
        log_file.flush().unwrap();

        // Sleep before next round (controls protocol frequency)
        thread::sleep(Duration::from_secs(5));
    }
}

/// Print the last known neighbors (from shared state)
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
