use anyhow::Context;
use futures_util::{SinkExt, StreamExt, stream};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use std::collections::HashMap;

pub const PROTOCOL_VERSION: i32 = 1;

// ─────────────────────────────────────────────
// Structuri de protocol (neschimbate față de original)
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub command: String,
    #[serde(default)]
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub id: i32,
    pub name: String,
    pub heroes: Vec<PlayerHeroSpawn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerHeroSpawn {
    pub id: i32,
    pub x: i32,
    pub y: i32,
    #[serde(rename = "type")]
    pub type_: String,
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
pub struct Projectile {
    pub owner_id: i32,
    #[serde(rename = "type")]
    pub type_: String,
    pub origin_x: i32,
    pub origin_y: i32,
    pub x: i32,
    pub y: i32,
    pub ttl: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wall {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    pub heroes: Vec<Hero>,
    pub projectiles: Vec<Projectile>,
    pub walls: Vec<Wall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartMatchArgs {
    pub config: GameConfig,
    pub state: GameState,
    pub match_id: String,
    pub your_player_id: i32,
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

// ─────────────────────────────────────────────
// Mesaj WebSocket generic
// ─────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct WsMsg {
    pub command: String,
    pub args: serde_json::Value,
}

/// Trimite un singur mesaj (LOGIN, PRACTICE, etc.)
async fn send_one<S>(write: &mut S, command: &str, args: serde_json::Value) -> anyhow::Result<()>
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let text = serde_json::to_string(&serde_json::json!({ "command": command, "args": args }))
        .context("serialize")?;
    write.send(Message::Text(text.into())).await.context("send")?;
    Ok(())
}

/// Trimite toate comenzile dintr-o tură într-un singur batch — ~2x mai rapid.
async fn send_all_commands<S>(
    write: &mut S,
    commands: Vec<(&str, serde_json::Value)>,
) -> anyhow::Result<()>
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let messages: Vec<Result<Message, tokio_tungstenite::tungstenite::Error>> = commands
        .into_iter()
        .map(|(cmd, args)| {
            let text = serde_json::to_string(&serde_json::json!({ "command": cmd, "args": args }))
                .expect("serialize command");
            Ok(Message::Text(text.into()))
        })
        .collect();

    write
        .send_all(&mut stream::iter(messages))
        .await
        .context("send_all")?;
    Ok(())
}

// ─────────────────────────────────────────────
// Starea curentă a meciului (ținută în memorie)
// ─────────────────────────────────────────────

struct MatchState {
    config: GameConfig,
    my_player_id: i32,
}

// ─────────────────────────────────────────────
// LOGICA DE JOC
// ─────────────────────────────────────────────

/// Returnează semnul unui număr (-1, 0 sau 1)
fn sign(v: i32) -> i32 {
    v.cmp(&0) as i32
}

/// Verifică dacă o poziție de centru (cx, cy) se suprapune cu un zid
fn overlaps_wall(cx: i32, cy: i32, walls: &[Wall]) -> bool {
    for w in walls {
        // Ambele sunt centre de blocuri 3x3; se suprapun dacă distanța < 3 pe oricare axă
        if (cx - w.x).abs() < 3 && (cy - w.y).abs() < 3 {
            return true;
        }
    }
    false
}

/// Calculează cel mai bun move pentru un erou față de o țintă
/// Întoarce coordonatele noi ale centrului
fn best_move(
    hero: &Hero,
    target_x: i32,
    target_y: i32,
    walls: &[Wall],
    map_w: i32,
    map_h: i32,
) -> (i32, i32) {
    let sx = sign(target_x - hero.x);
    let sy = sign(target_y - hero.y);

    // Direcțiile de încercat, în ordinea preferinței:
    // 1. diagonala ideală, 2. doar x, 3. doar y, 4. stai pe loc
    let candidates = [
        (sx, sy),
        (sx, 0),
        (0, sy),
        (0, 0), // no-op
    ];

    for (dx, dy) in candidates {
        if dx == 0 && dy == 0 {
            return (hero.x, hero.y); // no-op intenționat
        }
        let nx = hero.x + 3 * dx;
        let ny = hero.y + 3 * dy;
        // Verifică limite hartă (centrul trebuie să fie la cel puțin 1 tile de margine)
        if nx < 1 || ny < 1 || nx >= map_w - 1 || ny >= map_h - 1 {
            continue;
        }
        if !overlaps_wall(nx, ny, walls) {
            return (nx, ny);
        }
    }

    (hero.x, hero.y) // nu am putut muta, stăm pe loc
}

///calculul distantei cu chebysev
fn chebyshev(ax: i32, ay: i32, bx: i32, by: i32) -> i32 {
    (ax - bx).abs().max((ay - by).abs())
}

//verificare daca are linie libera
fn has_clear_line(from_x: i32, from_y: i32, to_x: i32, to_y: i32, walls: &[Wall]) -> bool {
    let mut x = from_x;
    let mut y = from_y;
    let dx = sign(to_x - from_x);
    let dy = sign(to_y - from_y);
    let steps = chebyshev(from_x, from_y, to_x, to_y);
    for _ in 0..steps {
        x += dx;
        y += dy;
        for w in walls {
            if (x - w.x).abs() <= 1 && (y - w.y).abs() <= 1 {
                return false;
            }
        }
    }
    true
}

//functie pentru a ne feri
fn escape_direction(hero: &Hero, projectiles: &[Projectile], my_player_id: i32) -> Option<(i32, i32)> {
    for p in projectiles {
        if p.owner_id == my_player_id { continue; }
        let dx = sign(p.x - p.origin_x);
        let dy = sign(p.y - p.origin_y);
        // Glonțul vine spre noi dacă e pe aceeași traiectorie și se apropie
        let on_path = sign(hero.x - p.x) == dx && sign(hero.y - p.y) == dy;
        let close = chebyshev(p.x, p.y, hero.x, hero.y) < 12;
        if on_path && close {
            // Fugi perpendicular: dacă glonțul merge pe Y → fugi pe X, și invers
            let ex = hero.x + if dy != 0 { 3 } else { 0 };
            let ey = hero.y + if dx != 0 { 3 } else { 0 };
            return Some((ex, ey));
        }
    }
    None
}

/// Procesează o tură: calculează toate acțiunile și le trimite într-un singur batch.
async fn process_turn<S>(
    write: &mut S,
    ms: &MatchState,
    turn_args: &StartTurnArgs,
) -> anyhow::Result<()>
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let state = &turn_args.state;
    let turn = turn_args.turn;

    let my_heroes: Vec<&Hero> = state.heroes.iter().filter(|h| h.owner_id == ms.my_player_id).collect();
    let enemy_heroes: Vec<&Hero> = state.heroes.iter().filter(|h| h.owner_id != ms.my_player_id).collect();

    println!("=== Tura {} | eroii mei: {} | inamici vizibili: {} ===",
        turn, my_heroes.len(), enemy_heroes.len());

    // construim lista de comenzi fără să trimitem nimic încă
    let mut commands: Vec<(&str, serde_json::Value)> = Vec::new();

    for (hero_index, hero) in my_heroes.iter().enumerate() {
        println!(
            "  Eroul {} la ({},{}) hp={} cooldown={}",
            hero.id, hero.x, hero.y, hero.hp, hero.cooldown
        );

        // ne ferim prioritate maxima
        if let Some((ex, ey)) = escape_direction(hero, &state.projectiles, ms.my_player_id) {
            println!("    → EVIT proiectil, MOVE ({ex},{ey})");
            commands.push(("MOVE", serde_json::json!({ "hero_id": hero.id, "x": ex, "y": ey })));
                continue;
        }

        // daca nu vedem inamicii ne apropiem de centru
        if enemy_heroes.is_empty() {
            let cx = ms.config.width / 2;
            let cy = ms.config.height / 2;
            let cx = cx - ((cx - 1) % 3);
            let cy = cy - ((cy - 1) % 3);
            let (nx, ny) = best_move(hero, cx, cy, &state.walls, ms.config.width, ms.config.height);
            println!("    → MOVE spre centru ({nx},{ny})");
            commands.push(("MOVE", serde_json::json!({ "hero_id": hero.id, "x": nx, "y": ny })));
            continue;
        }
         
        let assigned_target = enemy_heroes[hero_index % enemy_heroes.len()];
        let dist = chebyshev(hero.x, hero.y, assigned_target.x, assigned_target.y);

        if hero.cooldown == 0 
        {
            //daca poate trage
            if has_clear_line(hero.x, hero.y, assigned_target.x, assigned_target.y, &state.walls) {
            println!("    → SHOOT spre ({},{}) dist={}", assigned_target.x, assigned_target.y, dist);
                commands.push(("SHOOT", serde_json::json!({
                    "hero_id": hero.id,
                    "x": assigned_target.x,
                    "y": assigned_target.y
                })));
            }
            else {
                // nu putem trage ne apropiem de inamic
                let (nx, ny) = best_move(hero, assigned_target.x, assigned_target.y, &state.walls, ms.config.width, ms.config.height);
                commands.push(("MOVE", serde_json::json!({ "hero_id": hero.id, "x": nx, "y": ny })));
            }
        } else {
    const IDEAL_DIST: i32 = 12;
    let (nx, ny) = if dist > IDEAL_DIST + 3 {
        // prea depare ne apropiem
        best_move(hero, assigned_target.x, assigned_target.y, &state.walls, ms.config.width, ms.config.height)
    } else if dist < IDEAL_DIST - 3 {
        // prea aproape ne depărtăm
        let away_x = hero.x + 3 * sign(hero.x - assigned_target.x);
        let away_y = hero.y + 3 * sign(hero.y - assigned_target.y);
        (away_x, away_y)
    } else {
        (hero.x, hero.y) // distanta buna
    };
    commands.push(("MOVE", serde_json::json!({ "hero_id": hero.id, "x": nx, "y": ny })));
}
    }

    // Trimitem toate comenzile simultan într-un singur batch
    send_all_commands(write, commands).await?;

    Ok(())
}

// ─────────────────────────────────────────────
// MAIN
// ─────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = "wss://bitdefenders.cvjd.me/ws";
    println!("Conectare la {url} ...");

    let (ws, _) = connect_async(url).await.context("connect")?;
    let (mut write, mut read) = ws.split();
    println!("Conectat!");

    // Starea meciului curent (populată la START_MATCH)
    let mut match_state: Option<MatchState> = None;

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                println!("Eroare WebSocket: {e:?}");
                break;
            }
        };

        // Gestionăm Ping/Pong/Binary/Close la nivel de transport
        let text = match msg {
            Message::Text(t) => t,
            Message::Ping(payload) => {
                write.send(Message::Pong(payload)).await?;
                continue;
            }
            Message::Pong(_) => continue,
            Message::Binary(_) => {
                println!("Mesaj binar ignorat");
                continue;
            }
            Message::Close(frame) => {
                println!("Conexiune închisă: {frame:?}");
                break;
            }
            Message::Frame(_) => continue,
        };

        // Parsăm mesajul generic
        let msg: WsMsg = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                println!("Nu am putut parsa mesajul: {e}\nRaw: {text}");
                continue;
            }
        };

        println!("[server] command={}", msg.command);

        match msg.command.as_str() {
            // ── 1. Serverul se prezintă ──────────────────────────────────────
            "HELLO" => {
                println!("Server version: {}", msg.args["version"]);
                send_one(
                    &mut write,
                    "LOGIN",
                    serde_json::json!({
                        "name": "Portan-Patrick-bot",
                        "version": PROTOCOL_VERSION
                    }),
                )
                .await?;
                println!("LOGIN trimis");
            }

            // ── 2. Login acceptat ────────────────────────────────────────────
            "READY" => {
                println!("Suntem READY, pornim un meci de practică...");
                send_one(&mut write, "PRACTICE", serde_json::json!({})).await?;
            }

            // ── 3. Meciul începe ─────────────────────────────────────────────
            "START_MATCH" => {
                let args: StartMatchArgs =
                    serde_json::from_value(msg.args).context("parse START_MATCH")?;
                println!(
                    "Meci pornit! ID={} player_id={} harta={}x{}",
                    args.match_id,
                    args.your_player_id,
                    args.config.width,
                    args.config.height
                );
                println!("Eroi proprii la start:");
                for p in &args.config.players {
                    if p.id == args.your_player_id {
                        for h in &p.heroes {
                            println!("  Eroul {} tip={} la ({},{})", h.id, h.type_, h.x, h.y);
                        }
                    }
                }
                match_state = Some(MatchState {
                    my_player_id: args.your_player_id,
                    config: args.config,
                });
            }

            // ── 4. Fiecare tură ──────────────────────────────────────────────
            "START_TURN" => {
                let args: StartTurnArgs =
                    serde_json::from_value(msg.args).context("parse START_TURN")?;

                if let Some(ms) = &match_state {
                    if let Err(e) = process_turn(&mut write, ms, &args).await {
                        println!("Eroare în process_turn: {e}");
                    }
                } else {
                    println!("WARN: START_TURN primit fără match_state!");
                }
            }

            // ── 5. Meciul s-a terminat ───────────────────────────────────────
            "END_MATCH" => {
                let args: EndMatchArgs =
                    serde_json::from_value(msg.args).context("parse END_MATCH")?;
                match &args.winner {
                    Some(w) => println!("Meciul s-a terminat! Câștigător: {w} (motiv: {})", args.reason),
                    None => println!("Meciul s-a terminat cu rezultat egal! (motiv: {})", args.reason),
                }
                // Un singur meci per run — ieșim
                break;
            }

            // ── 6. Erori ─────────────────────────────────────────────────────
            "ERROR" => {
                let code = &msg.args["code"];
                let message = &msg.args["message"];
                let fatal = msg.args["fatal"].as_bool().unwrap_or(false);
                println!("ERROR [{code}]: {message} (fatal={fatal})");
                if fatal {
                    println!("Eroare fatală, ieșim.");
                    break;
                }
            }

            other => {
                println!("Comandă necunoscută: {other}");
            }
        }
    }

    println!("Deconectat.");
    Ok(())
}