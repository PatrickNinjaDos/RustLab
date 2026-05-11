use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use std::collections::HashMap;
use std::collections::VecDeque; // coada pentru BFS — echivalentul unui queue din C

pub const PROTOCOL_VERSION: i32 = 1;

// ─────────────────────────────────────────────────────────────────────────────
// STRUCTURI DE DATE
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerHeroSpawn {
    pub id: i32,
    pub x: i32,
    pub y: i32,
    #[serde(rename = "type")]
    pub type_: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub id: i32,
    pub name: String,
    pub heroes: Vec<PlayerHeroSpawn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeroTypeConfig {
    pub shoot_cooldown: i32,
    pub projectile_ttl: i32,
    pub projectile_speed: i32,
    pub max_hp: i32,
    pub projectile_damage: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameConfig {
    pub width: i32,
    pub height: i32,
    pub turns: i32,
    pub vision_range: i32,
    pub seed: u32,
    pub players: Vec<Player>,
    pub hero_types: HashMap<String, HeroTypeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hero {
    pub id: i32,
    pub owner_id: i32,
    #[serde(rename = "type")]
    pub type_: String,
    pub x: i32,
    pub y: i32,
    pub hp: i32,
    pub cooldown: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wall {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    pub heroes: Vec<Hero>,
    pub walls: Vec<Wall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartMatchArgs {
    pub match_id: String,
    pub your_player_id: i32,
    pub config: GameConfig,
    pub state: GameState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartTurnArgs {
    pub turn: i32,
    pub state: GameState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndMatchArgs {
    pub reason: String,
    #[serde(default)]
    pub winner: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WsMsg {
    pub command: String,
    pub args: serde_json::Value,
}

// ─────────────────────────────────────────────────────────────────────────────
// TRIMITERE MESAJE
// ─────────────────────────────────────────────────────────────────────────────

async fn send_msg<S>(write: &mut S, command: &str, args: serde_json::Value) -> anyhow::Result<()>
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let json = serde_json::json!({ "command": command, "args": args });
    let text = serde_json::to_string(&json).context("eroare serializare JSON")?;
    println!("  [TRIMIS] {}", text);
    write.send(Message::Text(text.into())).await.context("eroare trimitere mesaj")?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// HELPERS
// ─────────────────────────────────────────────────────────────────────────────

// verificare sa fie in harta
fn in_bounds(x: i32, y: i32, map_w: i32, map_h: i32) -> bool {
    x >= 1 && y >= 1 && x < map_w - 1 && y < map_h - 1
}

// verficare coliziune
fn overlaps_wall(cx: i32, cy: i32, walls: &[Wall]) -> bool {
    walls.iter().any(|w| (cx - w.x).abs() < 3 && (cy - w.y).abs() < 3)
}

// Snap: găsește cel mai aproape centru valid pe grila 3x3 față de (x, y).
// Centrele valide sunt pozițiile unde x % 3 == 1 și y % 3 == 1.
// Folosit ca să ne asigurăm că ținta e pe o poziție pe care un erou poate sta.
fn snap_to_grid(x: i32, y: i32) -> (i32, i32) {
    // Calculăm cel mai aproape număr cu rest 1 la împărțirea cu 3
    let snap = |v: i32| -> i32 {
        let r = v % 3;
        // r poate fi 0, 1, 2 (sau negativ în Rust, dar coordonatele noastre sunt pozitive)
        if r == 1 { v }        // deja pe grid
        else if r == 0 { v + 1 } // rotunjim în sus
        else { v - 1 }           // r == 2, rotunjim în jos
    };
    (snap(x), snap(y))
}

fn bfs_next_step(
    start_x: i32, start_y: i32,   
    target_x: i32, target_y: i32, 
    walls: &[Wall],
    map_w: i32, map_h: i32,
) -> (i32, i32) {

    //snap
    let (target_x, target_y) = snap_to_grid(target_x, target_y);

    //finish
    if start_x == target_x && start_y == target_y {
        println!("    [BFS] deja la țintă ({},{})", start_x, start_y);
        return (start_x, start_y);
    }

    // HashMap (cheie = poziție, valoare = poziția din care am venit).
    let mut came_from: HashMap<(i32, i32), (i32, i32)> = HashMap::new();

    // push_back = enqueue, pop_front = dequeue
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();

    came_from.insert((start_x, start_y), (start_x, start_y));
    queue.push_back((start_x, start_y)); 

    let directions: [(i32, i32); 8] = [
        ( 0,  3), // sus
        ( 0, -3), // jos
        ( 3,  0), // dreapta
        (-3,  0), // stânga
        ( 3,  3), // diagonal dreapta-sus
        ( 3, -3), // diagonal dreapta-jos
        (-3,  3), // diagonal stânga-sus
        (-3, -3), // diagonal stânga-jos
    ];

    while let Some((cx, cy)) = queue.pop_front() { 

        if cx == target_x && cy == target_y {
            // Urmăm came_from înapoi până ajungem la nodul al cărui tată e startul.
            // Acela e primul pas pe care trebuie să îl facem.
            let mut current = (cx, cy);
            loop {
                let parent = came_from[&current]; // de unde am venit în current
                if parent == (start_x, start_y) {
                    // current e primul pas după start — asta trimitem
                    println!("    [BFS] primul pas: ({},{}) → ({},{})",
                        start_x, start_y, current.0, current.1);
                    return current;
                }
                current = parent; // mergem un pas înapoi spre start
            }
        }

        for (dx, dy) in directions {
            let nx = cx + dx; // coordonata X a vecinului
            let ny = cy + dy; // coordonata Y a vecinului

            // Verificăm dacă vecinul e valid
            let valid = in_bounds(nx, ny, map_w, map_h) // e în hartă?
                && !overlaps_wall(nx, ny, walls)         // nu e zid?
                && !came_from.contains_key(&(nx, ny));   // nu l-am vizitat deja?

            if valid {
                came_from.insert((nx, ny), (cx, cy)); // marcăm de unde am venit
                queue.push_back((nx, ny));            // îl adăugăm în coadă
            }
        }
    }

    (start_x, start_y)
}

// ─────────────────────────────────────────────────────────────────────────────
// BRESENHAM 
// ─────────────────────────────────────────────────────────────────────────────
 
fn bresenham_line(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<(i32, i32)> {
    let mut points = Vec::new();
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    loop {
        points.push((x, y));
        if x == x1 && y == y1 { break; }
        let e2 = 2 * err;
        if e2 >= dy { err += dy; x += sx; }
        if e2 <= dx { err += dx; y += sy; }
    }
    points
}

// ─────────────────────────────────────────────────────────────────────────────
// LINE OF SIGHT
// ─────────────────────────────────────────────────────────────────────────────
 
// Returnează true dacă linia de la (x0,y0) la (x1,y1) nu e blocată de ziduri.
fn has_line_of_sight(x0: i32, y0: i32, x1: i32, y1: i32, walls: &[Wall]) -> bool {
    let line = bresenham_line(x0, y0, x1, y1);
    for (px, py) in line {
        for w in walls {
            if (px - w.x).abs() <= 1 && (py - w.y).abs() <= 1 {
                return false;
            }
        }
    }
    true
}

// ─────────────────────────────────────────────────────────────────────────────
// PROCESAREA TUREI
// ─────────────────────────────────────────────────────────────────────────────

// Gaseste prima pozitie libera de ziduri mergand de la fundul hartii in sus.
// Folosita ca target pentru eroi cand inamicul e in partea de jos.
fn find_bottom_target(spawn_x: i32, map_h: i32, walls: &[Wall], map_w: i32) -> (i32, i32) {
    let mut y = map_h - 2;
    while y >= 1 {
        let (sx, sy) = snap_to_grid(spawn_x, y);
        if in_bounds(sx, sy, map_w, map_h) && !overlaps_wall(sx, sy, walls) {
            return (sx, sy);
        }
        y -= 3;
    }
    snap_to_grid(spawn_x, map_h / 2)
}

// Gaseste prima pozitie libera de ziduri mergand de la varful hartii in jos.
fn find_top_target(spawn_x: i32, map_h: i32, walls: &[Wall], map_w: i32) -> (i32, i32) {
    let mut y = 1;
    while y < map_h - 1 {
        let (sx, sy) = snap_to_grid(spawn_x, y);
        if in_bounds(sx, sy, map_w, map_h) && !overlaps_wall(sx, sy, walls) {
            return (sx, sy);
        }
        y += 3;
    }
    snap_to_grid(spawn_x, map_h / 2)
}

async fn process_turn<S>(
    write: &mut S,
    my_player_id: i32,
    config: &GameConfig,
    map_walls: &[Wall],   
    target_x: &mut i32,
    target_y: &mut i32,
    home_x: i32,
    home_y: i32,
    away_x: i32,
    away_y: i32,
    turn_args: &StartTurnArgs,
) -> anyhow::Result<()>
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let state = &turn_args.state;
    let map_w = config.width;
    let map_h = config.height;

    let my_heroes: Vec<&Hero> = state.heroes.iter()
        .filter(|h| h.owner_id == my_player_id)
        .collect();

    let enemy_heroes: Vec<&Hero> = state.heroes.iter()
        .filter(|h| h.owner_id != my_player_id)
        .collect();

    // Colectam toate mesajele turei si le trimitem odata cu send_all
    let mut messages: Vec<Message> = Vec::new();

    for hero in &my_heroes {

        if hero.x == *target_x && hero.y == *target_y {
            if *target_x == away_x && *target_y == away_y {
                *target_x = home_x;
                *target_y = home_y;
            } else {
                *target_x = away_x;
                *target_y = away_y;
            }
        }

        // tragem daca se poate
        if hero.cooldown == 0 && !enemy_heroes.is_empty() {
            let target = enemy_heroes.iter().find(|enemy| {
                enemy.cooldown == 1
                    && has_line_of_sight(hero.x, hero.y, enemy.x, enemy.y, map_walls)
            });

            if let Some(enemy) = target {
                let json = serde_json::json!({
                    "command": "SHOOT",
                    "args": {
                        "hero_id": hero.id,
                        "x": enemy.x,
                        "y": enemy.y,
                        "comment": "🔫"
                    }
                });
                messages.push(Message::Text(serde_json::to_string(&json).unwrap().into()));
                continue;
            }
        }

        let (move_x, move_y) = bfs_next_step(
            hero.x, hero.y,
            *target_x, *target_y,
            map_walls,
            map_w, map_h,
        );

        let comment = if move_x == hero.x && move_y == hero.y {
            "😴"
        } else {
            "🚶"
        };

        let json = serde_json::json!({
            "command": "MOVE",
            "args": {
                "hero_id": hero.id,
                "x": move_x,
                "y": move_y,
                "comment": comment
            }
        });
        messages.push(Message::Text(serde_json::to_string(&json).unwrap().into()));
    }

    println!("  [SEND_ALL] {} mesaje", messages.len());
    write.send_all(&mut futures_util::stream::iter(messages).map(Ok)).await
        .context("eroare send_all")?;

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// MAIN
// ─────────────────────────────────────────────────────────────────────────────
pub const VERSUS_PLAYERS: bool = false;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = "wss://bitdefenders.cvjd.me/ws";
    println!("Conectare la {url} ...");

    let (ws, _) = connect_async(url).await.context("nu s-a putut conecta")?;
    let (mut write, mut read) = ws.split();
    println!("Conectat!");

    let mut config: Option<GameConfig> = None;
    let mut my_player_id: i32 = 0;

    let mut target_x: i32 = 0;
    let mut target_y: i32 = 0;
    let mut home_x: i32 = 0;
    let mut home_y: i32 = 0;
    let mut away_x: i32 = 0;
    let mut away_y: i32 = 0;

    // Zidurile complete ale hărții — salvate o singură dată la START_MATCH.
    // La fiecare tură folosim acestea pentru BFS, nu state.walls (care e cu fog).
    let mut map_walls: Vec<Wall> = Vec::new();

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => { println!("Eroare WebSocket: {e:?}"); break; }
        };

        let text = match msg {
            Message::Text(t) => t,
            Message::Ping(payload) => { write.send(Message::Pong(payload)).await?; continue; }
            Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => continue,
            Message::Close(frame) => { println!("Conexiune închisă: {frame:?}"); break; }
        };

        let msg: WsMsg = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => { println!("Parse error: {e}\nRaw: {text}"); continue; }
        };

        println!("[SERVER] → {}", msg.command);

        match msg.command.as_str() {
            "HELLO" => {
                send_msg(&mut write, "LOGIN", serde_json::json!({
                    "name": "Portan-Patrick-bot",
                    "version": PROTOCOL_VERSION
                })).await?;
            }
            "READY" => {
                if VERSUS_PLAYERS {
                    send_msg(&mut write, "CHALLENGE", serde_json::json!({})).await?;
                } else {
                    send_msg(&mut write, "PRACTICE", serde_json::json!({
                        "my_id": 1
                    })).await?;
                }
            }
            "START_MATCH" => {
                let args: StartMatchArgs = serde_json::from_value(msg.args)
                    .context("eroare la parsarea START_MATCH")?;

                println!("Meci pornit! ID={} player_id={} hartă={}x{}",
                    args.match_id, args.your_player_id,
                    args.config.width, args.config.height);

                // Salvăm zidurile hărții complete — le folosim pentru tot meciul
                println!("  ziduri pe hartă: {}", args.state.walls.len());
                map_walls = args.state.walls; // <-- salvăm harta completă aici

                my_player_id = args.your_player_id;

                // Calculam home si away din spawn-urile eroilor
                let map_w = args.config.width;
                let map_h = args.config.height;
                let my_spawn_x = args.config.players.iter()
                    .find(|p| p.id == my_player_id)
                    .and_then(|p| p.heroes.first())
                    .map(|h| h.x)
                    .unwrap_or(map_w / 2);
                let my_spawn_y = args.config.players.iter()
                    .find(|p| p.id == my_player_id)
                    .and_then(|p| p.heroes.first())
                    .map(|h| h.y)
                    .unwrap_or(0);
                let enemy_spawn_x = args.config.players.iter()
                    .find(|p| p.id != my_player_id)
                    .and_then(|p| p.heroes.first())
                    .map(|h| h.x)
                    .unwrap_or(map_w / 2);
                // home = cea mai apropiata pozitie libera de la varful hartii
                // away = cea mai apropiata pozitie libera de la fundul hartii
                let we_are_at_bottom = my_spawn_y > map_h / 2;
                let (hx, hy) = if we_are_at_bottom {
                    find_bottom_target(my_spawn_x, map_h, &map_walls, map_w)
                } else {
                    find_top_target(my_spawn_x, map_h, &map_walls, map_w)
                };
                let (ax, ay) = if we_are_at_bottom {
                    find_top_target(enemy_spawn_x, map_h, &map_walls, map_w)
                } else {
                    find_bottom_target(enemy_spawn_x, map_h, &map_walls, map_w)
                };
                home_x = hx; home_y = hy;
                away_x = ax; away_y = ay;
                target_x = away_x;
                target_y = away_y;
                println!("  [INIT] home=({},{}) away=({},{})", home_x, home_y, away_x, away_y);

                config = Some(args.config);

            }
            "START_TURN" => {
                let args: StartTurnArgs = serde_json::from_value(msg.args)
                    .context("eroare la parsarea START_TURN")?;

                if let Some(cfg) = &config {
                    if let Err(e) = process_turn(
                        &mut write,
                        my_player_id,
                        cfg,
                        &map_walls,
                        &mut target_x,
                        &mut target_y,
                        home_x,
                        home_y,
                        away_x,
                        away_y,
                        &args,
                    ).await {
                        println!("Eroare în process_turn: {e}");
                    }
                }
            }
            "END_MATCH" => {
                let args: EndMatchArgs = serde_json::from_value(msg.args)
                    .context("eroare la parsarea END_MATCH")?;
                match &args.winner {
                    Some(w) => println!("Câștigător: {w} (motiv: {})", args.reason),
                    None    => println!("Egalitate (motiv: {})", args.reason),
                }
                break;
            }
            "ERROR" => {
                let fatal = msg.args["fatal"].as_bool().unwrap_or(false);
                println!("EROARE server: {} (fatal={fatal})", msg.args["message"]);
                if fatal { break; }
            }
            other => println!("Comandă necunoscută: {other}"),
        }
    }

    println!("Deconectat.");
    Ok(())
}