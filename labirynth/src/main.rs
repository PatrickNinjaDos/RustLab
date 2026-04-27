use std::env;
use std::io;
use std::fs::File;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Point {
    x: usize,
    y: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CellType {
    Wall,
    Path,
}

#[derive(Debug, Deserialize)]
struct GridCell {
    #[serde(rename = "type")]
    cell_type: CellType,
    x: usize,
    y: usize,
}

#[derive(Debug, Deserialize)]
struct Labyrinth {
    width: usize,
    height: usize,
    start: Point,
    goal: Point,
    grid: Vec<GridCell>,
}

fn visualize(labyrinth: &Labyrinth) {
    // 1 = perete, 0 = drum
    let mut grid = vec![vec![false; labyrinth.width]; labyrinth.height];

    for cell in &labyrinth.grid {
        if let CellType::Wall = cell.cell_type {
            grid[cell.y][cell.x] = true;
        }
    }

    println!();
    for y in 0..labyrinth.height {
        for x in 0..labyrinth.width {
            if x == labyrinth.start.x && y == labyrinth.start.y {
                print!("S"); 
            } else if x == labyrinth.goal.x && y == labyrinth.goal.y {
                print!("G"); 
            } else if grid[y][x] != false {
                print!("█"); 
            } else {
                print!(" "); 
            }
        }
        println!();
    }
    println!();
    println!("Legenda: S = Start({},{}), G = Goal({},{}), █ = Perete, ' ' = Drum liber",
        labyrinth.start.x, labyrinth.start.y,
        labyrinth.goal.x, labyrinth.goal.y);
}

fn parcurgere_labyrinth(
    labyrinth: &Labyrinth,
    grid: &Vec<Vec<bool>>,
    visited: &mut Vec<Vec<bool>>,
    current: (usize, usize),
    step: usize,
) {
    let (x, y) = current;

    //FINAL
    if x == labyrinth.goal.x && y == labyrinth.goal.y {
        println!("am ajuns la destinatie in {} pasi", step);
        return;
    }

    // SUD
    if y + 1 < labyrinth.height && grid[y + 1][x] == false && visited[y + 1][x] == false {
        visited[y + 1][x] = true;
        parcurgere_labyrinth(labyrinth, grid, visited, (x, y + 1), step + 1);
    }

    // WEST
    if x + 1 < labyrinth.width && grid[y][x + 1] == false && visited[y][x + 1] == false {
        visited[y][x + 1] = true;
        parcurgere_labyrinth(labyrinth, grid, visited, (x + 1, y), step + 1);
    }

    // NORD
    if y > 0 && grid[y - 1][x] == false && visited[y - 1][x] == false {
        visited[y - 1][x] = true;
        parcurgere_labyrinth(labyrinth, grid, visited, (x, y - 1), step + 1);
    }

    // VEST
    if x > 0 && grid[y][x - 1] == false && visited[y][x - 1] == false {
        visited[y][x - 1] = true;
        parcurgere_labyrinth(labyrinth, grid, visited, (x - 1, y), step + 1);
    }
}
fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    println!("uite astea sunt argumentele: {:?}", args);
 
    let file = File::open(&args[1])?;
    println!("am deschis fisierul: {:?}", file);
 
    let labyrinth: Labyrinth = serde_json::from_reader(file)
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    visualize(&labyrinth);

    let mut grid_matrix = vec![vec![false; labyrinth.width]; labyrinth.height];
    for cell in &labyrinth.grid {
        if let CellType::Wall = cell.cell_type {
            grid_matrix[cell.y][cell.x] = true;
        }
    }

    let mut visited = vec![vec![false; labyrinth.width]; labyrinth.height];
    let start = (labyrinth.start.x, labyrinth.start.y);
    visited[start.1][start.0] = true;

    parcurgere_labyrinth(&labyrinth, &grid_matrix, &mut visited, start, 0);
 
    Ok(())
}