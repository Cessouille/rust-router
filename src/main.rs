use hostname::get;
use std::collections::{HashMap, HashSet};

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
            // Find a neighbor router that is connected to both a network we have and the target network
            let mut next_hop = None;
            for iface in &router.interfaces {
                if let Some(neighbors) = network_map.get(iface.network) {
                    for &(neighbor_name, neighbor_ip) in neighbors {
                        if neighbor_name == router.name {
                            continue;
                        }
                        // Only add route if neighbor is DIRECTLY connected to the destination network
                        if let Some(neigh_ifaces) = network_map.get(network) {
                            if neigh_ifaces
                                .iter()
                                .any(|(n_name, _)| *n_name == neighbor_name)
                            {
                                next_hop = Some((network, neighbor_ip));
                                break;
                            }
                        }
                    }
                }
                if next_hop.is_some() {
                    break;
                }
            }
            if let Some((dest_net, via_ip)) = next_hop {
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
                println!("No direct neighbor route found for {}", network);
            }
        }
    } else {
        println!("Hostname '{}' does not match any router.", hostname);
    }
}
