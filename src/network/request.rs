use crate::audio::{play_music_by_vec, save_music_to_disk, MusicPlayer};
use crate::interface::Peer;
use crate::network::handshake::{
    json_string_to_network_table, send_change_name_request, send_network_table_request,
    send_table_request, send_table_to_all_peers, update_table_after_delete,
};
use crate::network::music_exchange::{
    delete_redundant_song_request, read_file_exist, send_exist_response, send_file_request,
    send_get_file_reponse, song_order_request,
};
use crate::network::{
    other_random_target, send_local_file_status, send_read_request, send_status_request,
    send_write_request,
};
use crate::utils::FileInstructions::{GET, ORDER, PLAY, REMOVE};
use crate::utils::FileStatus::{DELETE, NEW};
use crate::utils::{AppListener, FileInstructions};
use std::net::SocketAddr;
use std::process;
use std::time::SystemTime;

pub fn push_to_db(
    key: String,
    value: Vec<u8>,
    peer: &mut Peer,
    listener: &mut Box<dyn AppListener + Sync>,
) {
    if peer.database.data.contains_key(&key) {
        println!("File already exists in your database");
    } else {
        peer.process_store_request((key.clone(), value.clone()));
        println!("Saved file to database");
        let key_clone = key.clone();
        listener.local_database_changed(key_clone, NEW);

        let redundant_target = other_random_target(&peer.network_table, peer.get_ip());
        match redundant_target {
            Some(target) => {
                send_write_request(
                    target,
                    *peer.get_ip(),
                    (key.clone(), value),
                    true,
                    peer,
                );
                match peer.redundancy_table.get_mut(&target) {
                    Some(p) => p.push(key),
                    None => {
                        peer.redundancy_table.insert(target.clone(), vec![key]);
                    }
                }
            }
            None => println!("Only peer in network. No redundancy possible"),
        };
    }
}

pub fn redundant_push_to_db(
    key: String,
    value: Vec<u8>,
    peer: &mut Peer,
    listener: &mut Box<dyn AppListener + Sync>,
    from: String,
) {
    let key_clone = key.clone();
    let key_redundant_clone = key.clone();
    peer.process_store_request((key, value));
    listener.local_database_changed(key_clone, NEW);
    let from_address = match from.parse::<SocketAddr>() {
        Ok(a) => a,
        Err(_e) => {
            error!("Could not parse senders address to SocketAddr");
            return;
        }
    };

    match peer.redundancy_table.get_mut(&from_address) {
        Some(p) => p.push(key_redundant_clone),
        None => {
            peer.redundancy_table
                .insert(from_address, vec![key_redundant_clone]);
        }
    }
}

pub fn change_peer_name(value: String, sender: SocketAddr, peer: &mut Peer) {
    peer.network_table.remove(&peer.name);
    peer.name = value;
    peer.network_table
        .insert(peer.name.clone(), peer.ip_address);
    //send request existing network table
    send_table_request(sender, *peer.get_ip(), &peer.name);
}

pub fn send_network_table(value: Vec<u8>, peer: &mut Peer) {
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

pub fn send_network_update_table(value: Vec<u8>, peer: &mut Peer) {
    let table = match String::from_utf8(value) {
        Ok(val) => val,
        Err(utf) => {
            dbg!(utf);
            return;
        }
    };
    let new_network_peer = json_string_to_network_table(table);
    for (key, addr) in new_network_peer {
        let name = key.clone();
        peer.network_table.insert(key, addr);
        println!("{} joined the network.", name);
    }
}

pub fn request_for_table(value: String, sender: SocketAddr, peer: &mut Peer) {
    // checks if key is unique, otherwise send change name request
    if peer.network_table.contains_key(&value) {
        let name = format!("{}+{}", &value, "1");
        send_change_name_request(sender, *peer.get_ip(), name.as_ref());
    } else {
        send_network_table_request(sender, &peer);
    }
}

pub fn find_file(
    instr: FileInstructions,
    song_name: String,
    peer: &mut Peer,
    listener: &mut Box<dyn AppListener + Sync>,
) {
    // @TODO there is no feedback when audio does not exist in "global" database (there is only the existsFile response, when file exists in database? change?
    // @TODO in this case we need to remove the request?
    if peer.get_db().get_data().contains_key(&song_name) {
        if instr == REMOVE {
            peer.delete_file_from_database(&song_name);
            let song_clone = song_name.clone();
            listener.local_database_changed(song_clone, DELETE);
            println!("Remove file {} from database", &song_name);

            let id = SystemTime::now();
            peer.add_new_request(&id, instr);

            for (_key, value) in &peer.network_table {
                if _key != &peer.name {
                    delete_redundant_song_request(*value, peer.ip_address, &song_name);
                }
            }
        } else if instr == GET {
            if let Some(file) = peer.get_db().get_data().get(&song_name) {
                if let Err(e) = save_music_to_disk(file.clone(), &song_name) {
                    error!("{}", e);
                }
            }
        }
    } else {
        let id = SystemTime::now();
        peer.add_new_request(&id, instr);

        for (_key, value) in &peer.network_table {
            if _key != &peer.name {
                read_file_exist(*value, peer.ip_address, &song_name, id);
            }
        }
    }
}

pub fn get_file(instr: FileInstructions, key: String, sender: SocketAddr, peer: &mut Peer) {
    match peer.find_file(key.as_ref()) {
        Some(music) => {
            send_get_file_reponse(sender, peer.ip_address, key.as_ref(), music.clone(), instr)
        }
        None => {
            //@TODO error handling}
            println!("TODO!");
        }
    }
}

pub fn get_file_response(
    instr: &FileInstructions,
    key: &str,
    value: Vec<u8>,
    peer: &mut Peer,
    sink: &mut MusicPlayer,
) -> Result<(), String> {
    match instr {
        PLAY => {
            //save to tmp and play audio
            play_music_by_vec(value, sink, key.to_string())
        }
        GET => {
            if let Err(_e) = save_music_to_disk(value, &key.to_string()) {
                return Err("Could not save music to disk".to_string());
            };
            Ok(())
        }
        ORDER => {
            peer.process_store_request((key.to_string(), value));
            Ok(())
        }
        _ => Err("Unknown command".to_string()),
    }
}

pub fn exist_file(song_name: String, id: SystemTime, sender: SocketAddr, peer: &mut Peer) {
    let exist = peer.does_file_exist(song_name.as_ref());
    if exist {
        send_exist_response(sender, peer.ip_address, song_name.as_ref(), id);
    }
}

pub fn exit_peer(addr: SocketAddr, peer: &mut Peer) {
    if peer.network_table.len() > 1 {
        for value in peer.network_table.values() {
            if *value != addr {
                update_table_after_delete(*value, addr, &peer.name);
            }
        }
        let database = peer.get_db().get_data();
        let network_table = &peer.network_table;
        if network_table.len() > 1 {
            for song in database.keys() {
                let redundant_target = match other_random_target(network_table, peer.get_ip()) {
                    Some(r) => r,
                    None => {
                        continue;
                    }
                };
                song_order_request(redundant_target, peer.ip_address, song.to_string());
            }
        }
    }
    process::exit(0);
}

pub fn delete_from_network(name: String, peer: &mut Peer) {
    if peer.network_table.contains_key(&name) {
        peer.network_table.remove(&name);
        println!("{} left the network.", &name);
    }
}

pub fn exist_file_response(song_name: String, id: SystemTime, sender: SocketAddr, peer: &mut Peer) {
    //Check if peer request is still active. when true remove it
    let peer_clone = peer.open_request_table.clone();
    match peer_clone.get(&id) {
        Some(instr) => {
            peer.delete_handled_request(&id);
            send_file_request(sender, peer.ip_address, song_name.as_ref(), instr.clone());
        }
        None => {
            info!("Did not find requested file");
        }
    }
}

pub fn status_request(sender: SocketAddr, peer: &mut Peer) {
    let mut res: Vec<String> = Vec::new();
    for k in peer.get_db().data.keys() {
        res.push(k.to_string());
    }
    let peer_name = &peer.name;
    send_local_file_status(sender, res, *peer.get_ip(), peer_name.to_string());
}

pub fn self_status_request(peer: &mut Peer) {
    let mut cloned_peer = peer.clone();
    for addr in peer.network_table.values() {
        send_status_request(*addr, *peer.get_ip(), &mut cloned_peer);
    }
}

pub fn dropped_peer(addr: SocketAddr, peer: &mut Peer) {
    println!("Peer at {:?} was dropped", addr);
    peer.drop_peer_by_ip(&addr);

    redistribute_files(addr, peer);
}

pub fn order_song_request(song_name: String, peer: &mut Peer) {
    let network_table = &peer.network_table;
    if peer.get_db().get_data().contains_key(&song_name) {
        let redundant_target = match other_random_target(network_table, peer.get_ip()) {
            Some(r) => r,
            None => {
                error!("Could not find a redundant target");
                return;
            }
        };
        song_order_request(redundant_target, peer.ip_address, song_name);
    } else {
        send_read_request(peer, &song_name, FileInstructions::ORDER)
    }
}

pub fn delete_file_request(song_name: &str, peer: &mut Peer) {
    if peer.database.data.contains_key(song_name) {
        println!("Remove file {} from database", &song_name);
        peer.delete_file_from_database(song_name);
    }
}

pub fn redistribute_files(addr: SocketAddr, peer: &mut Peer) {
    let mut peer_clone = peer.clone();
    if peer.network_table.len() > 1 {
        //let database = peer.get_db().get_data();
        let redundant_table = &peer.redundancy_table;
        let network_table = &peer.network_table;
        let song_list = match redundant_table.get(&addr) {
            Some(s) => s,
            None => {
                return;
            }
        };
        if network_table.len() > 1 {
            for song in song_list {
                let redundant_target = match other_random_target(network_table, peer.get_ip()) {
                    Some(r) => r,
                    None => {
                        continue;
                    }
                };
                let file = match peer.find_file(song) {
                    Some(f) => f,
                    None => {return;}
                };
                send_write_request(redundant_target, peer.ip_address, (song.to_string(), file.clone()),true, &mut peer_clone );
            }
        }
        peer.redundancy_table.remove(&addr);
    }
}
