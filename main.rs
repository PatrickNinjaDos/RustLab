use std::env;
use std::io;
use std::fs::File;

use serde::Deserialize;

#[derive(Debug,Deserialize)]

struct GridCell{
    x: usize,
    y: usize,
}

#[derive(Debug,Deserialize)]

struct Labyrinth {
    width: usize,
    height: usize,
    start: usize,
    goal: usize,
    grid: Vec<GridCell>,
}

fn main() -> io::Result<()> {
    
    let args: Vec<String> = env::args().collect();
    println!("uite astea sunt argumentele: {:?}",args);

    let file = File::open(&args[1])?;
    println!("am deschis fisierul: {:?}",file);

    let labyrinth: Labyrinth = serde_json::from_reader(file)?;
    println!("labirintul este: {:?}",labyrinth);

    Ok(())
}
