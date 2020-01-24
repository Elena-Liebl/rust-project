use std::io::Read;
use std::net::TcpListener;
use std::net::{SocketAddr, TcpStream};
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::JoinHandle;

mod handshake;
mod music_exchange;
mod notification;
pub mod peer;
mod response;

extern crate get_if_addrs;
extern crate rand;
use rand::Rng;

use crate::audio::{play_music, play_music_by_vec};
use crate::constants::HEARTBEAT_SLEEP_DURATION;
use crate::network::handshake::{
    json_string_to_network_table, send_change_name_request, send_network_table, send_table_request,
    send_table_to_all_peers, update_table_after_delete,
};
use crate::network::music_exchange::{
    read_file_exist, send_exist_response, send_file_request, send_get_file_reponse,
};
use crate::network::notification::*;
use crate::network::peer::{create_peer, Peer};
use crate::network::response::*;
use crate::shell::{print_external_files, spawn_shell};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::SystemTime;

#[cfg(target_os = "macos")]
pub fn get_own_ip_address(port: &str) -> Result<SocketAddr, String> {
    let ifs = match get_if_addrs::get_if_addrs() {
        Ok(v) => v,
        Err(_e) => return Err("Failed to find any network address".to_string()),
    };
    let if_options = ifs
        .into_iter()
        .find(|i| i.name == "en0".to_string() && i.addr.ip().is_ipv4());
    let this_ipv4: String = if let Some(interface) = if_options {
        interface.addr.ip().to_string()
    } else {
        "Local ip address not found".to_string()
    };
    println!("Local IP Address: {}", this_ipv4);
    let ipv4_port = format!("{}:{}", this_ipv4, port);
    let peer_socket_addr = match ipv4_port.parse::<SocketAddr>() {
        Ok(val) => val,
        Err(_e) => return Err("Could not parse ip address to SocketAddr".to_string()),
    };
    Ok(peer_socket_addr)
}

// This function only gets compiled if the target OS is linux
#[cfg(not(target_os = "macos"))]
pub fn get_own_ip_address(port: &str) -> Result<SocketAddr, String> {
    let this_ipv4 = match local_ipaddress::get() {
        Some(val) => val,
        None => return Err("Failed to find any network address".to_string()),
    };
    println!("Local IP Address: {}", this_ipv4);
    let ipv4_port = format!("{}:{}", this_ipv4, port);
    let peer_socket_addr = match ipv4_port.parse::<SocketAddr>() {
        Ok(val) => val,
        Err(e) => return Err("Could not parse ip address to SocketAddr".to_string()),
    };
    Ok(peer_socket_addr)
}

pub fn startup(name: String, port: String) -> JoinHandle<()> {
    let concurrent_thread = thread::Builder::new().name("ConThread".to_string());
    concurrent_thread
        .spawn(move || {
            let peer = create_peer(name.as_ref(), port.as_ref()).unwrap();
            let peer_arc = Arc::new(Mutex::new(peer));
            let peer_arc_clone_listen = peer_arc.clone();
            let listener = thread::Builder::new()
                .name("TCPListener".to_string())
                .spawn(move || {
                    match listen_tcp(peer_arc_clone_listen) {
                        Ok(_) => {}
                        Err(_) => {
                            eprintln!("Failed to spawn listener");
                        }
                    };
                })
                .unwrap();
            let peer_arc_clone_interact = peer_arc.clone();
            let interact = thread::Builder::new()
                .name("Interact".to_string())
                .spawn(move || {
                    match spawn_shell(peer_arc_clone_interact) {
                        Ok(_) => {}
                        Err(_) => {
                            eprintln!("Failed to spawn shell");
                        }
                    };
                })
                .unwrap();
            let peer_arc_clone_heartbeat = peer_arc.clone();
            let heartbeat = thread::Builder::new()
                .name("Heartbeat".to_string())
                .spawn(move || match start_heartbeat(peer_arc_clone_heartbeat) {
                    Ok(_) => {}
                    Err(_) => {
                        eprintln!("Failed to spawn shell");
                    }
                })
                .unwrap();
            listener.join().expect_err("Could not join Listener");
            interact.join().expect_err("Could not join Interact");
            heartbeat.join().expect_err("Could not join Heartbeat");
        })
        .unwrap()
}

pub fn join_network(own_name: &str, port: &str, ip_address: SocketAddr) -> Result<(), String> {
    let peer = create_peer(own_name, port).unwrap();
    let own_addr = peer.ip_address;
    let peer_arc = Arc::new(Mutex::new(peer));
    let peer_arc_clone_listen = peer_arc.clone();

    let listener = thread::Builder::new()
        .name("TCPListener".to_string())
        .spawn(move || {
            match listen_tcp(peer_arc_clone_listen) {
                Ok(_) => {}
                Err(_) => {
                    eprintln!("Failed to spawn listener");
                }
            };
        })
        .unwrap();
    let peer_arc_clone_interact = peer_arc.clone();
    let peer_arc_clone_heartbeat = peer_arc.clone();

    //send request existing network table
    send_table_request(&ip_address, &own_addr, own_name);

    let interact = thread::Builder::new()
        .name("Interact".to_string())
        .spawn(move || {
            //spawn shell
            match spawn_shell(peer_arc_clone_interact) {
                Ok(_) => {}
                Err(_) => {
                    eprintln!("Failed to spawn shell");
                }
            };
        })
        .unwrap();
    let heartbeat = thread::Builder::new()
        .name("Heartbeat".to_string())
        .spawn(move || match start_heartbeat(peer_arc_clone_heartbeat) {
            Ok(_) => {}
            Err(_) => {
                eprintln!("Failed to spawn shell");
            }
        })
        .unwrap();
    listener.join().expect_err("Could not join Listener");
    interact.join().expect_err("Could not join Interact");
    heartbeat.join().expect_err("Could not join Heartbeat");
    Ok(())
}

fn listen_tcp(arc: Arc<Mutex<Peer>>) -> Result<(), String> {
    let clone = arc.clone();
    let listen_ip = clone.lock().unwrap().ip_address;
    let listener = TcpListener::bind(&listen_ip).unwrap();
    println!("Listening on {}", listen_ip);
    for stream in listener.incoming() {
        let mut buf = String::new();
        //        dbg!(&stream);
        match stream {
            Ok(mut s) => {
                s.read_to_string(&mut buf).unwrap();
                //                let deserialized: SendRequest = match serde_json::from_str(&buf) {
                //                    Ok(val) => val,
                //                    Err(e) => {
                //                        dbg!(e);
                //                        println!("Could not deserialize {:?}", &buf);
                //                        continue; // skip this stream
                //                    }
                //                };
                let des: Notification = match serde_json::from_str(&buf) {
                    Ok(val) => val,
                    Err(e) => {
                        dbg!(e);
                        println!("Could not deserialize {:?}", &buf);
                        continue; // skip this stream
                    }
                };
                let mut peer = clone.lock().unwrap();
                //                dbg!(&deserialized);
                handle_notification(des, &mut peer);
                //                handle_incoming_requests(deserialized, &mut peer);
                drop(peer);
                println!("Request handled.");
                // TODO: Response, handle duplicate key, redundancy
            }
            Err(_e) => {
                println!("could not read stream");
                return Err("Error".to_string());
            }
        };
    }
    Ok(())
}

fn start_heartbeat(arc: Arc<Mutex<Peer>>) -> Result<(), String> {
    loop {
        thread::sleep(HEARTBEAT_SLEEP_DURATION);
        let mut peer = arc.lock().unwrap();
        let mut peer_clone = peer.clone();
        drop(peer);
        let network_size = peer_clone.network_table.len();
        if network_size == 1 {
            continue;
        } else if network_size < 4 {
            send_heartbeat(&mut peer_clone)
        } else {
            // TODO: send to n < network_size targets
        }
    }
    Ok(())
}

fn send_heartbeat(peer: &mut Peer) {
    let mut cloned_peer = peer.clone();
    for (_k, addr) in &peer.network_table {
        match TcpStream::connect(addr) {
            Ok(_) => {}
            Err(_e) => {
                handle_lost_connection(*addr, &mut cloned_peer);
            }
        }
    }
}

fn handle_notification(notification: Notification, peer: &mut Peer) {
    //dbg!(&notification);
    let sender = notification.from;
    match notification.content {
        Content::PushToDB { key, value, from } => {
            peer.process_store_request((key.clone(), value.clone()));
            let redundant_target = other_random_target(&peer.network_table, peer.get_ip());
            match redundant_target {
                Some(target) => {
                    send_write_request(
                        target,
                        *peer.get_ip(),
                        (key.clone(), value.clone()),
                        true,
                        peer,
                    );
                }
                None => println!("Only peer in network. No redundancy possible"),
            };
            match from.parse::<SocketAddr>() {
                Ok(target_address) => {
                    send_write_response(target_address, *peer.get_ip(), key.clone(), peer);
                }
                Err(e) => {
                    dbg!(e);
                }
            }
        }
        Content::RedundantPushToDB { key, value, .. } => {
            peer.process_store_request((key, value));
        }
        Content::ChangePeerName { value } => {
            peer.network_table.remove(&peer.name);
            peer.name = value;
            peer.network_table
                .insert(peer.name.clone(), peer.ip_address);
            //send request existing network table
            send_table_request(
                &SocketAddr::from_str(&sender.to_string()).unwrap(),
                peer.get_ip(),
                &peer.name,
            );
        }
        Content::SendNetworkTable { value } => {
            let table = match String::from_utf8(value) {
                Ok(val) => val,
                Err(utf) => {
                    dbg!(utf);
                    return;
                }
            };
            let network_table = json_string_to_network_table(table);
            for (key, addr) in network_table {
                peer.network_table.insert(key, addr);
            }
            send_table_to_all_peers(peer);
        }
        Content::SendNetworkUpdateTable { value } => {
            let table = match String::from_utf8(value) {
                Ok(val) => val,
                Err(utf) => {
                    dbg!(utf);
                    return;
                }
            };
            let new_network_peer = json_string_to_network_table(table);
            for (key, addr) in new_network_peer {
                peer.network_table.insert(key, addr);
            }
        }
        Content::RequestForTable { value } => {
            // checks if key is unique, otherwise send change name request
            if peer.network_table.contains_key(&value) {
                let name = format!("{}+{}", &value, "1");
                send_change_name_request(sender.to_string(), peer.get_ip(), name.as_ref());
            } else {
                send_network_table(sender.to_string(), &peer);
            }
        }
        Content::FindFile { key } => {
            // @TODO check if file is in database first
            // @TODO there is no feedback when audio does not exist in "global" database (there is only the existsFile response, when file exists in database? change?
            // @TODO in this case we need to remove the request?
            let id = SystemTime::now();
            peer.add_new_request(&id, &key);

            for (_key, value) in &peer.network_table {
                if _key != &peer.name {
                    read_file_exist(*value, peer.ip_address, &key, id.clone());
                }
            }
        }
        Content::ExistFile { id, key } => {
            let exist = peer.does_file_exist(key.as_ref());
            if exist {
                send_exist_response(sender, peer.ip_address, key.as_ref(), id);
            }
        }
        Content::ExistFileResponse { key, id } => {
            //Check if peer request is still active. when true remove it
            if peer.check_request_still_active(&id) {
                //@TODO maybe create new request?
                peer.delete_handled_request(&id);
                send_file_request(sender, peer.ip_address, key.as_ref());
            }
        }
        Content::GetFile { key } => {
            match peer.find_file(key.as_ref()) {
                Some(music) => {
                    send_get_file_reponse(sender, peer.ip_address, key.as_ref(), music.clone())
                }
                None => {
                    //@TODO error handling}
                }
            }
        }
        Content::GetFileResponse { value, .. } => {
            //save to tmp and play audio
            if peer.waiting_to_play {
                peer.waiting_to_play = false;
                match play_music_by_vec(&value) {
                    Ok(_) => {}
                    Err(_) => {
                        eprintln!("Failed to play music");
                    }
                };
            }
            //Download mp3 file
        }
        Content::Response { .. } => {}
        Content::ExitPeer { addr } => {
            for value in peer.network_table.values() {
                if *value != addr {
                    update_table_after_delete(*value, addr, &peer.name);
                }
            }
            process::exit(0);
        }
        Content::DeleteFromNetwork { name } => {
            if peer.network_table.contains_key(&name) {
                peer.network_table.remove(&name);
            }
        }
        Content::SelfStatusRequest {} => {
            let mut cloned_peer = peer.clone();
            for addr in peer.network_table.values() {
                send_status_request(*addr, *peer.get_ip(), &mut cloned_peer);
            }
        }
        Content::StatusRequest {} => {
            let mut res: Vec<String> = Vec::new();
            for k in peer.get_db().data.keys() {
                res.push(k.to_string());
            }
            let peer_name = &peer.name;
            send_local_file_status(sender, res, *peer.get_ip(), peer_name.to_string());
        }
        Content::StatusResponse { files, name } => {
            print_external_files(files, name);
        }
        Content::PlayAudioRequest { name } => match play_music(peer, name.as_str()) {
            Ok(_) => {}
            Err(e) => {
                println!("{}", e);
            }
        },
        Content::DroppedPeer { addr } => {
            peer.find_peer_by_ip(&addr);
        }
    }
}

pub fn send_write_request(
    target: SocketAddr,
    origin: SocketAddr,
    data: (String, Vec<u8>),
    redundant: bool,
    peer: &mut Peer,
) {
    let stream = match TcpStream::connect(target) {
        Ok(s) => s,
        Err(_e) => {
            handle_lost_connection(target, peer);
            return;
        }
    };
    if let true = redundant {
        let not = Notification {
            content: Content::RedundantPushToDB {
                key: data.0,
                value: data.1,
                from: origin.to_string(),
            },
            from: origin,
        };
        match serde_json::to_writer(&stream, &not) {
            Ok(ser) => ser,
            Err(_e) => {
                println!("Failed to serialize SendRequest {:?}", &not);
            }
        };
    } else {
        let not = Notification {
            content: Content::PushToDB {
                key: data.0,
                value: data.1,
                from: origin.to_string(),
            },
            from: origin,
        };
        match serde_json::to_writer(&stream, &not) {
            Ok(ser) => ser,
            Err(_e) => {
                println!("Failed to serialize SendRequest {:?}", &not);
            }
        };
    }
}

fn other_random_target(
    network_table: &HashMap<String, SocketAddr>,
    own_ip: &SocketAddr,
) -> Option<SocketAddr> {
    if network_table.len() == 1 {
        return None;
    }
    let mut rng = rand::thread_rng();
    let mut index = rng.gen_range(0, network_table.len());
    let mut target = network_table.values().skip(index).next().unwrap();
    while target == own_ip {
        index = rng.gen_range(0, network_table.len());
        target = network_table.values().skip(index).next().unwrap();
    }
    Some(*target)
}

pub fn send_write_response(target: SocketAddr, origin: SocketAddr, key: String, peer: &mut Peer) {
    let stream = match TcpStream::connect(target) {
        Ok(s) => s,
        Err(_e) => {
            handle_lost_connection(target, peer);
            return;
        }
    };

    let not = Notification {
        content: Content::Response {
            from: origin,
            message: Message::DataStored { key },
        },
        from: origin,
    };
    match serde_json::to_writer(&stream, &not) {
        Ok(ser) => ser,
        Err(_e) => {
            println!("Failed to serialize Response {:?}", &not);
        }
    };
}

pub fn send_read_request(target: SocketAddr, name: &str) {
    /// Communicate to the listener that we want to find the location of a given file
    let stream = match TcpStream::connect(target) {
        Ok(s) => s,
        Err(_e) => {
            return;
        }
    };

    let not = Notification {
        content: Content::FindFile {
            key: name.to_string(),
        },
        from: target,
    };

    match serde_json::to_writer(&stream, &not) {
        Ok(ser) => ser,
        Err(_e) => {
            println!("Failed to serialize SendRequest {:?}", &not);
        }
    };
}

pub fn send_delete_peer_request(target: SocketAddr) {
    let stream = match TcpStream::connect(target) {
        Ok(s) => s,
        Err(_e) => {
            return;
        }
    };

    let not = Notification {
        content: Content::ExitPeer { addr: target },
        from: target,
    };

    match serde_json::to_writer(&stream, &not) {
        Ok(ser) => ser,
        Err(_e) => {
            println!("Failed to serialize SendRequest {:?}", &not);
        }
    };
}

pub fn send_self_status_request(target: SocketAddr, peer: &mut Peer) {
    let stream = match TcpStream::connect(target) {
        Ok(s) => s,
        Err(_e) => {
            handle_lost_connection(target, peer);
            return;
        }
    };

    let not = Notification {
        content: Content::SelfStatusRequest {},
        from: target,
    };

    match serde_json::to_writer(&stream, &not) {
        Ok(ser) => ser,
        Err(_e) => {
            println!("Failed to serialize SendRequest {:?}", &not);
        }
    };
}

pub fn send_status_request(target: SocketAddr, from: SocketAddr, peer: &mut Peer) {
    let stream = match TcpStream::connect(target) {
        Ok(s) => s,
        Err(_e) => {
            handle_lost_connection(target, peer);
            return;
        }
    };

    let not = Notification {
        content: Content::StatusRequest {},
        from,
    };

    match serde_json::to_writer(&stream, &not) {
        Ok(ser) => ser,
        Err(_e) => {
            println!("Failed to serialize SendRequest {:?}", &not);
        }
    };
}

fn send_local_file_status(
    target: SocketAddr,
    files: Vec<String>,
    from: SocketAddr,
    peer_name: String,
) {
    let stream = match TcpStream::connect(target) {
        Ok(s) => s,
        Err(_e) => {
            return;
        }
    };
    let not = Notification {
        content: Content::StatusResponse {
            files,
            name: peer_name,
        },
        from,
    };

    match serde_json::to_writer(&stream, &not) {
        Ok(ser) => ser,
        Err(_e) => {
            println!("Failed to serialize SendRequest {:?}", &not);
        }
    };
}

pub fn send_play_request(name: &str, from: SocketAddr) {
    let stream = match TcpStream::connect(from) {
        Ok(s) => s,
        Err(_e) => {
            //            handle_lost_connection(from, peer); TODO
            return;
        }
    };
    let not = Notification {
        content: Content::PlayAudioRequest {
            name: name.to_string(),
        },
        from,
    };
    match serde_json::to_writer(&stream, &not) {
        Ok(ser) => ser,
        Err(_e) => {
            println!("Failed to serialize SendRequest {:?}", &not);
        }
    };
}

fn handle_lost_connection(addr: SocketAddr, peer: &mut Peer) {
    peer.find_peer_by_ip(&addr);
    let mut cloned_peer = peer.clone();
    // TODO: Send notification to other peers that this peer was dropped
    for (_, other_addr) in &peer.network_table {
        send_dropped_peer_notification(*other_addr, addr, &mut cloned_peer)
    }
    println!("TODO");
}

fn send_dropped_peer_notification(target: SocketAddr, dropped_addr: SocketAddr, peer: &mut Peer) {
    let stream = match TcpStream::connect(target) {
        Ok(s) => s,
        Err(_e) => {
            handle_lost_connection(target, peer);
            return;
        }
    };
    let not = Notification {
        content: Content::DroppedPeer { addr: dropped_addr },
        from: *peer.get_ip(),
    };
    match serde_json::to_writer(&stream, &not) {
        Ok(ser) => ser,
        Err(_e) => {
            println!("Failed to serialize SendRequest {:?}", &not);
        }
    };
}
