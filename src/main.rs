fn main() {
    use std::collections::HashSet;
    use std::net::{SocketAddrV4, UdpSocket};

    let port = 9999;
    let hello_msg = b"HELLO";
    let _neighbors: HashSet<std::net::Ipv4Addr> = HashSet::new();

    // Spawn a thread to listen for hello packets
    let listener = std::thread::spawn(move || {
        let listen_socket =
            UdpSocket::bind(("0.0.0.0", port)).expect("Failed to bind listen socket");
        listen_socket.set_broadcast(true).unwrap();
        let mut buf = [0u8; 64];
        let start = std::time::Instant::now();
        let mut discovered = HashSet::new();
        while start.elapsed().as_secs() < 2 {
            if let Ok((size, src)) = listen_socket.recv_from(&mut buf) {
                if &buf[..size] == hello_msg {
                    if let std::net::SocketAddr::V4(src_v4) = src {
                        discovered.insert(src_v4.ip().clone());
                    }
                }
            }
        }
        discovered
    });

    // Broadcast hello on each interface
    let interfaces = pnet::datalink::interfaces();
    for iface in interfaces {
        for ip in iface.ips {
            if let pnet::ipnetwork::IpNetwork::V4(ipv4) = ip {
                let local_ip = ipv4.ip();
                let broadcast_ip = ipv4.broadcast();
                let socket = UdpSocket::bind((local_ip, 0)).expect("Failed to bind socket");
                socket.set_broadcast(true).unwrap();
                let dest = SocketAddrV4::new(broadcast_ip, port);
                socket.send_to(hello_msg, dest).ok();
                println!("Sent hello from {} to {}", local_ip, broadcast_ip);
            }
        }
    }

    // Wait for neighbor discovery to finish
    let discovered = listener.join().unwrap();
    println!("Discovered neighbors: {:?}", discovered);

    // Add a route for each discovered neighbor (example: add route to their /24 network)
    for neighbor_ip in discovered {
        // Example: assume neighbor's network is /24
        let net = format!(
            "{}.{}.{}.0/24",
            neighbor_ip.octets()[0],
            neighbor_ip.octets()[1],
            neighbor_ip.octets()[2]
        );
        let via = neighbor_ip.to_string();
        println!("Adding route: ip route add {} via {}", net, via);
        // let status = std::process::Command::new("ip")
        //     .args(&["route", "add", &net, "via", &via])
        //     .status();
        // match status {
        //     Ok(s) if s.success() => println!("  Route added successfully."),
        //     Ok(s) => eprintln!("  Failed to add route (exit code {}).", s),
        //     Err(e) => eprintln!("  Error executing ip route: {}", e),
        // }
    }
}
