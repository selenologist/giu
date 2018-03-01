use notify::{DebouncedEvent, Watcher, RecursiveMode, watcher};
use subprocess::{Exec, ExitStatus};

use std::sync::mpsc::channel;
use std::time::Duration;
use std::io;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::thread::{Thread,JoinHandle};

fn process_coffee(path: &PathBuf) -> io::Result<()>{
    println!("[rebuilder] Compiling {}", path.to_str().unwrap());
    let exit_status = match
        Exec::cmd("coffee")
        .arg("-c")
        .arg("-M")
        .arg(path)
        .join(){
            Ok(k) => k,
            Err(e) => return Err(io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))
    };

    if exit_status != ExitStatus::Exited(0){
        println!("[rebuilder] Error, returned {:?}", exit_status);
        Err(io::Error::new(io::ErrorKind::Other, format!("exited with status {:?}", exit_status)))
    }
    else{
        println!("[rebuilder] {} processed.", path.to_str().unwrap());
        Ok(())
    }
}

fn recursive_find(path: &Path) -> io::Result<()>{
    println!("[rebuilder] Entering {}", path.to_str().unwrap());
    for p in fs::read_dir(path)?{
        let e = p.unwrap();
        println!("Found file {:?}", e.file_name());
        if e.file_type().unwrap().is_dir(){
            recursive_find(&e.path());
        }
        if e.file_type().unwrap().is_file(){
            let path = e.path();
            if path.extension().unwrap() == "coffee"{
               process_coffee(&path)?;
            }
        }
    }
    Ok(())
}

fn handle_event(event: DebouncedEvent){
    use self::DebouncedEvent::*;
    match event{
        Create(p) |
        Write(p)  |
        Rename(.., p) => {
            println!("File {} modified", p.to_str().unwrap());
            if p.extension().unwrap() == "coffee"{
                match process_coffee(&p){
                    Ok(..) => {},
                    Err(e) => println!("Failed to process: {:?}", e)
                }
            }
        },
        _ => {}
    }
}

pub fn launch_thread() -> JoinHandle<()>{
    thread::spawn(move ||{
        let watch_path = "client/";
        println!("[rebuilder] Finding and processing existing .coffee files");
        recursive_find(Path::new(watch_path));

        let (tx, rx) = channel();

        let mut watcher = watcher(tx, Duration::from_secs(5)).unwrap();

        watcher.watch(watch_path, RecursiveMode::Recursive).unwrap();

        loop{
            match rx.recv(){
                Ok(ev) => handle_event(ev),
                Err(e) => println!("[rebuilder] watch error: {:?}", e)
            }
        }
    })
}
