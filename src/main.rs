use hostname::get;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone)]
struct Interface {
    network: &'static str,
    ip: &'static str,
}

#[derive(Debug, Clone)]
struct Router {
    name: &'static str,
    interfaces: Vec<Interface>,
}

fn main() {
    // Define routers and their interfaces
    let routers = vec![
        Router {
            name: "R_1",
            interfaces: vec![
                Interface {
                    network: "192.168.1.0/24",
                    ip: "192.168.1.1",
                },
                Interface {
                    network: "10.1.0.0/24",
                    ip: "10.1.0.1",
                },
            ],
        },
        Router {
            name: "R_2",
            interfaces: vec![
                Interface {
                    network: "192.168.2.0/24",
                    ip: "192.168.2.1",
                },
                Interface {
                    network: "10.1.0.0/24",
                    ip: "10.1.0.2",
                },
                Interface {
                    network: "10.2.0.0/24",
                    ip: "10.2.0.2",
                },
            ],
        },
        Router {
            name: "R_3",
            interfaces: vec![
                Interface {
                    network: "192.168.3.0/24",
                    ip: "192.168.3.1",
                },
                Interface {
                    network: "10.3.0.0/24",
                    ip: "10.3.0.3",
                },
            ],
        },
        Router {
            name: "R_4",
            interfaces: vec![
                Interface {
                    network: "10.1.0.0/24",
                    ip: "10.1.0.4",
                },
                Interface {
                    network: "10.2.0.0/24",
                    ip: "10.2.0.4",
                },
            ],
        },
        Router {
            name: "R_5",
            interfaces: vec![
                Interface {
                    network: "10.2.0.0/24",
                    ip: "10.2.0.5",
                },
                Interface {
                    network: "10.3.0.0/24",
                    ip: "10.3.0.5",
                },
            ],
        },
    ];

    // Get the hostname and normalize it (lowercase, remove domain)
    let hostname = get().unwrap_or_default().to_string_lossy().to_lowercase();
    let hostname = hostname.split('.').next().unwrap_or("").to_string();

    // Find the router that matches the hostname (e.g., "r1" for "R_1")
    let router_opt = routers.iter().find(|r| {
        let router_id = r.name.to_lowercase().replace("_", "");
        router_id == hostname
    });

    if let Some(router) = router_opt {
        // Build a map of network -> router/interface
        let mut network_map: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
        for r in &routers {
            for iface in &r.interfaces {
                network_map
                    .entry(iface.network)
                    .or_default()
                    .push((r.name, iface.ip));
            }
        }

        // Build a map of router name -> router
        let router_map: HashMap<&str, &Router> = routers.iter().map(|r| (r.name, r)).collect();

        // Collect all networks
        let all_networks: HashSet<&str> = network_map.keys().cloned().collect();

        println!("Routes for {}:", router.name);
        let directly_connected: HashSet<&str> = router
            .interfaces
            .iter()
            .map(|iface| iface.network)
            .collect();

        for &network in &all_networks {
            if directly_connected.contains(network) {
                continue;
            }

            // BFS to find the next hop router for this network
            let mut visited: HashSet<&str> = HashSet::new();
            let mut queue: VecDeque<(&str, Option<&str>, Option<&str>)> = VecDeque::new();
            // (current_router, first_hop_ip, via_iface_ip)
            queue.push_back((router.name, None, None));
            let mut found = None;

            while let Some((current_router, first_hop_ip, _via_iface_ip)) = queue.pop_front() {
                if !visited.insert(current_router) {
                    continue;
                }
                let current = router_map.get(current_router).unwrap();
                // If current router is directly connected to the destination network
                if current
                    .interfaces
                    .iter()
                    .any(|iface| iface.network == network)
                {
                    if let Some(ip) = first_hop_ip {
                        found = Some((network, ip));
                    }
                    break;
                }
                // Enqueue neighbors
                for iface in &current.interfaces {
                    if let Some(neighbors) = network_map.get(iface.network) {
                        for &(neighbor_name, neighbor_ip) in neighbors {
                            if neighbor_name == current_router {
                                continue;
                            }
                            // The first hop from the original router
                            let next_first_hop_ip = if current_router == router.name {
                                Some(neighbor_ip)
                            } else {
                                first_hop_ip
                            };
                            queue.push_back((neighbor_name, next_first_hop_ip, Some(iface.ip)));
                        }
                    }
                }
            }

            if let Some((dest_net, via_ip)) = found {
                println!("Adding route: ip route add {} via {}", dest_net, via_ip);
                let status = std::process::Command::new("ip")
                    .args(&["route", "add", dest_net, "via", via_ip])
                    .status();
                match status {
                    Ok(s) if s.success() => println!("  Route added successfully."),
                    Ok(s) => eprintln!("  Failed to add route (exit code {}).", s),
                    Err(e) => eprintln!("  Error executing ip route: {}", e),
                }
            } else {
                println!("No route found for {}", network);
            }
        }
    } else {
        println!("Hostname '{}' does not match any router.", hostname);
    }
}
