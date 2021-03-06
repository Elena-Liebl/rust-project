use prettytable::format;
extern crate colored;
use colored::*;
use meff::interface::{Peer, upload_music, music_request, delete_peer, music_control};
use meff::utils::FileInstructions::{GET, REMOVE};
use std::borrow::BorrowMut;
use std::convert::TryFrom;
use std::error::Error;
use std::io::stdin;
use std::sync::{Arc, Mutex};
use std::thread;
use crate::util::Application;
use meff::interface::MusicState::{PAUSE, STOP, CONTINUE, PLAY};

pub fn spawn_shell(arc: Arc<Mutex<Peer>>, model: Arc<Mutex<Application>>) -> Result<(), Box<dyn Error>> {
    let arc_clone = arc.clone();
    let arc_clone2 = arc.clone();
    let peer = match arc.lock() {
        Ok(p) => p,
        Err(e) => e.into_inner()
    };

    drop(peer);
    let handle = match thread::Builder::new()
        .name("Interaction".to_string())
        .spawn(move || loop {
            let peer = match arc_clone.lock() {
                Ok(p) => p,
                Err(e) => e.into_inner(),
            };
            drop(peer);
            handle_user_input(&arc_clone2, &model);
        }) {
        Ok(h) => h,
        Err(_) => {
            error!("Failed to spawn thread");
            return Err(Box::try_from("Failed to spwan thread".to_string()).unwrap());
        }
    };
    handle.join().unwrap();
    Ok(())
}

pub fn handle_user_input(arc: &Arc<Mutex<Peer>>, model: &Arc<Mutex<Application>>) {
    loop {
        let peer = match arc.lock() {
            Ok(p) => p,
            Err(e) => e.into_inner(),
        };
        let model = match model.lock() {
            Ok(m) => m,
            Err(e) => e.into_inner(),
        };
        let model_clone = model.clone();
        let mut peer_clone = peer.clone();
        drop(peer);
        drop(model);
        let buffer = &mut String::new();
        if let Err(e) = stdin().read_line(buffer) {
            error!("Failed to handle user input {:?}", e);
        };
        let _ = buffer.trim_end();
        let buffer_iter = buffer.split_whitespace();
        let instructions: Vec<&str> = buffer_iter.collect();
        match instructions.first() {
            Some(&"h") => {
                show_help_instructions();
            }
            Some(&"help") => {
                show_help_instructions();
            }
            Some(&"push") => {
                if instructions.len() == 3 {
                    match upload_music(
                        instructions[1],
                        instructions[2],
                        peer_clone.ip_address,
                        peer_clone.borrow_mut(),
                    ) {
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("Failed to push {} to database", instructions[1]);
                            error!(
                                "Could not push {:?} to the database, error code {:?}",
                                instructions, e
                            );
                        }
                    };
                } else {
                    println!(
                        "You need to specify name and filepath. For more information type help.\n"
                    );
                }
            }
            Some(&"get") => {
                if instructions.len() == 2 {
                    music_request(&mut peer_clone, instructions[1], GET);
                } else {
                    println!(
                        "You need to specify name and filepath. For more information type help.\n"
                    );
                }
            }
            Some(&"exit") => {
                println!("You are leaving the network.");
                delete_peer(&mut peer_clone);
            }
            Some(&"status") => {
                print_peer_status(&arc);
                print_local_db_status(&arc);
            }
            Some(&"play") => {
                if instructions.len() == 2 {
                    music_control(Some(instructions[1].to_string()), &mut peer_clone, PLAY);
                } else if *model_clone.is_playing.lock().unwrap() {
                        music_control(None, &mut peer_clone, CONTINUE);
                    } else {
                        println!("File name is missing. For more information type help.\n");
                    }
                }
            Some(&"remove") => {
                if instructions.len() == 2 {
                    music_request(&mut peer_clone, instructions[1], REMOVE);
                } else {
                    println!(
                        "You need to specify name of mp3 file. For more information type help.\n"
                    );
                }
            }
            Some(&"pause") => {
                music_control(None, &mut peer_clone, PAUSE);
            }
            Some(&"stop") => {
                music_control(None, &mut peer_clone, STOP);
            }
            _ => println!("No valid instructions. Try help!\n"),
        }
    }
}


pub fn show_help_instructions() {
    let info = "\nHelp Menu:\n\n\
                Use following instructions: \n\n\
                status - show current state of peer\n\
                push [mp3 name] [direction to mp3] - add mp3 to database\n\
                get [mp3 name] - get mp3 file from database\n\
                remove [mp3 name] - deletes mp3 file from database\n\
                play [mp3 name] - plays the audio of mp3 file\n\
                exit - exit network and leave program\n\n
                ";
    print!("{}", info);
}

fn print_peer_status(arc: &Arc<Mutex<Peer>>) {
    let peer = match arc.lock() {
        Ok(p) => p,
        Err(e) => e.into_inner(),
    };
    let peer_clone = peer.clone();
    drop(peer);
    let nwt = peer_clone.network_table;
    let mut other_peers = table!(["Name".italic().yellow(), "SocketAddr".italic().yellow()]);

    for (name, addr) in nwt {
        other_peers.add_row(row![name, addr.to_string()]);
    }
    other_peers.set_format(*format::consts::FORMAT_BORDERS_ONLY);
    println!(
        "\n\n{}\n{}",
        "Current members in the network"
            .to_string()
            .black()
            .on_white(),
        other_peers
    );
}

/// Print the current status of the local database
/// # Arguments:
/// * `peer` - the local `Peer`
fn print_local_db_status(arc: &Arc<Mutex<Peer>>) {
    let peer = match arc.lock() {
        Ok(p) => p,
        Err(e) => e.into_inner(),
    };
    let peer_clone = peer.clone();
    drop(peer);
    let db = peer_clone.get_db().get_data();
    let mut local_data = table!(["Key".italic().green(), "File Info".italic().green()]);
    for (k, v) in db {
        local_data.add_row(row![k, v.len()]);
    }
    local_data.set_format(*format::consts::FORMAT_BORDERS_ONLY);
    print!(
        "\n\n{}\n{}",
        "Current state of local database".to_string(),
        local_data
    );
}

/// Print the name of all files from another peer
/// # Arguments
/// * `files` - `Vec<String>` of filenames from another peer
/// * `peer_name` - the name of the peer that holds the files
pub fn print_external_files(files: Vec<String>, peer_name: String) {
    let mut table = table!(["Key".italic().green()]);
    for k in files {
        table.add_row(row![k]);
    }
    table.set_format(*format::consts::FORMAT_BORDERS_ONLY);
    let text = format!(
        "{} {}",
        "Files stored in peer ".to_string().black().on_white(),
        peer_name
    );
    println!("\n\n{}\n{}", text, table);
}
