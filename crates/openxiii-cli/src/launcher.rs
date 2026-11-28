//! oxiii launcher: equivalent of ServerLauncher.exe (XIII's "Dedicated Server Launcher"), windowless. The original
//! launcher builds the UCC command line ("server map?options -port=N") then hands it to ShellExecuteA("ucc");
//! here the same line goes to oxiii server.

use std::path::Path;

/// Modes, map prefix, GameInfo class, mutator.
pub const MODES: [(&str, &str, &str, &str); 6] = [
    ("Deathmatch", "DM", "XIIIMP.XIIIMPGameInfo", ""),
    ("Team Deathmatch", "DM", "XIIIMP.XIIIMPTeamGameInfo", ""),
    ("Capture The Flag", "CT", "XIIIMP.XIIIMPCTFGameInfo", ""),
    ("The Hunt", "DM", "XIIIMP.XIIIMPDuckGameInfo", ""),
    ("Sabotage", "SB", "XIIIMP.XIIIMPBombGame", ""),
    ("Power Up", "DM", "XIIIMP.XIIIMPGameInfo", "XIIIMP.MarioMutator"),
];
/// Score limits and time limits in minutes of the launcher lists; 0 = NoLimit.
pub const POINTS: [i32; 19] = [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 150, 200, 250, 300, 350, 400, 450, 500];
pub const TIME: [i32; 13] = [0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60];

/// One map of the list (0x48 bytes): display name (+0), file (+0x20), players (+0x40),
/// bDeathMatchOnly (+0x44).
#[derive(Clone, Debug, PartialEq)]
pub struct Map {
    pub readable: String,
    pub file_value: String,
    pub players: i32,
    pub dm_single: bool,
}

fn value_value<'a>(line_value: &'a str, key_tuple: &str) -> Option<&'a str> {
    let at = line_value.find(&format!("{key_tuple}="))? + key_tuple.len() + 1;
    let rest = &line_value[at..];
    Some(match rest.strip_prefix('"') {
        Some(q) => q.split('"').next().unwrap_or(""),
        None => rest.split([',', ')']).next().unwrap_or(""),
    })
}

/// Maps from MapList.ini with bOnPC, then those of ..\MapsUser\*.unr:
/// "XX_Name_N.unr" gives the display name "Name" (between the first and second "_", or up to the dot) and
/// N players (8 without a second "_").
pub fn maps_list(game_value: &Path) -> Vec<Map> {
    let mut v = Vec::new();
    let text_value = std::fs::read_to_string(openxiii_package::system_dir(&game_value).join("MapList.ini")).unwrap_or_default();
    for l in text_value.lines().filter(|l| l.trim_start().starts_with("MapListInfo=")) {
        let true_value = |k: &str| value_value(l, k).is_some_and(|x| x.eq_ignore_ascii_case("true"));
        if !true_value("bOnPC") {
            continue;
        }
        v.push(Map {
            readable: value_value(l, "MapReadableName").unwrap_or("").to_string(),
            file_value: value_value(l, "MapUnrName").unwrap_or("").to_string(),
            players: value_value(l, "NbPlayers").and_then(|x| x.trim().parse().ok()).unwrap_or(0),
            dm_single: true_value("bDeathMatchOnly"),
        });
    }
    let mut character: Vec<String> = std::fs::read_dir(game_value.join("MapsUser"))
        .map(|d| d.flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    character.retain(|f| f.to_ascii_lowercase().ends_with(".unr"));
    character.sort();
    for f in character {
        v.push(map_character(&f));
    }
    v
}

fn map_character(file_value: &str) -> Map {
    let start_value = file_value.find('_').map_or(0, |i| i + 1);
    let rest = &file_value[start_value..];
    let (readable, players) = match rest.find('_') {
        Some(i) => (&rest[..i], crate_atoi(&rest[i + 1..])),
        None => (rest.split('.').next().unwrap_or(rest), 8),
    };
    Map { readable: readable.to_string(), file_value: file_value.to_string(), players, dm_single: false }
}

/// CRT atoi: whitespace, sign, digits.
fn crate_atoi(s: &str) -> i32 {
    let s = s.trim_start();
    let (neg, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let n = s.bytes().take_while(u8::is_ascii_digit).fold(0i32, |a, c| a.wrapping_mul(10).wrapping_add((c - b'0') as i32));
    if neg { n.wrapping_neg() } else { n }
}

/// Whether a map is listed for the mode (first two letters of the file, case-sensitive):
/// Deathmatch takes every DM; otherwise the mode prefix without bDeathMatchOnly; Team Deathmatch also takes CT.
pub fn map_mode(c: &Map, mode: usize) -> bool {
    let p = c.file_value.get(..2).unwrap_or("");
    (mode == 0 && p == "DM") || (p == MODES[mode].1 && !c.dm_single) || (mode == 1 && p == "CT")
}

/// Launcher window settings.
pub struct Settings {
    pub mode: usize,
    pub map_value: Map,
    pub players: i32,
    pub shots_friends: bool,
    pub points: i32,
    pub time_value: i32,
    pub name_value: String,
    pub port: String,
    /// None: local network; Some((account, password, private password)): ubi.com.
    pub ubi: Option<(String, String, Option<String>)>,
}

/// Launcher admin password: 10 to 29 characters drawn from "0-9a-z"; the same text
/// is used as AdminPassword and Password.
pub fn admin_password(mut rand: impl FnMut() -> i32) -> String {
    const C: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let n = rand() % 20 + 10;
    (0..n).map(|_| C[(rand() % 36) as usize] as char).collect()
}

/// The UCC command line: spaces in the display name become "_", an empty
/// server name becomes "NoNameServer".
pub fn line_value(r: &Settings, password: &str) -> String {
    let (_, _, game_value, mutator) = MODES[r.mode];
    let map_value = r.map_value.readable.replace(' ', "_");
    let sn = if r.name_value.is_empty() { "NoNameServer" } else { r.name_value.as_str() };
    let (idx, ff, fr, ti) = (r.mode, u8::from(r.shots_friends), r.points, r.time_value);
    let (file_value, np, port) = (&r.map_value.file_value, r.players, &r.port);
    match &r.ubi {
        None => format!("server {file_value}?LAN?GameIdx={idx}?MapName={map_value}?NP={np}?FF={ff}?FR={fr}?TI={ti}?Game={game_value}?SN={sn}?AdminPassword={password}?Password={password}?Mutator={mutator} -port={port}"),
        Some((alias, gsmdp, None)) => format!("server {file_value}?GameIdx={idx}?MapName={map_value}?NP={np}?FF={ff}?FR={fr}?TI={ti}?GS=1?GSAlias={alias}?GSPassword={gsmdp}?ServerName={sn}?IsPrivate=N?Game={game_value}?AdminPassword={password}?Password={password}?SN={sn}?Mutator={mutator} -port={port}"),
        Some((alias, gsmdp, Some(private_value))) => format!("server {file_value}?GameIdx={idx}?MapName={map_value}?NP={np}?FF={ff}?FR={fr}?TI={ti}?GS=1?GSAlias={alias}?GSPassword={gsmdp}?ServerName={sn}?IsPrivate=Y?GamePassword={private_value}?Game={game_value}?AdminPassword={password}?Password={password}?SN={sn}?Mutator={mutator} -port={port}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_maps_user() {
        assert_eq!(map_character("DM_My Map_6.unr"), Map { readable: "My Map".into(), file_value: "DM_My Map_6.unr".into(), players: 6, dm_single: false });
        assert_eq!(map_character("CTF_Base.unr").readable, "Base");
        assert_eq!(map_character("CTF_Base.unr").players, 8);
        assert_eq!(map_character("Plain.unr").readable, "Plain");
    }

    #[test]
    fn filter_modes() {
        let c = |f: &str, dm| Map { readable: String::new(), file_value: f.into(), players: 4, dm_single: dm };
        assert!(map_mode(&c("DM_Hual04a.unr", true), 0));
        assert!(!map_mode(&c("DM_Hual04a.unr", true), 3));
        assert!(map_mode(&c("DM_Hual1.unr", false), 5));
        assert!(map_mode(&c("CTF_Base.unr", false), 1));
        assert!(map_mode(&c("CTF_Base.unr", false), 2));
        assert!(!map_mode(&c("dm_x.unr", false), 0));
        assert!(map_mode(&c("SB_Camp.unr", false), 4));
    }

    #[test]
    fn lines_list() {
        let r = Settings {
            mode: 5,
            map_value: Map { readable: "Winslow Bank".into(), file_value: "DM_Banque.unr".into(), players: 6, dm_single: false },
            players: 6,
            shots_friends: true,
            points: 20,
            time_value: 10,
            name_value: String::new(),
            port: "7777".into(),
            ubi: None,
        };
        assert_eq!(line_value(&r, "abc"), "server DM_Banque.unr?LAN?GameIdx=5?MapName=Winslow_Bank?NP=6?FF=1?FR=20?TI=10?Game=XIIIMP.XIIIMPGameInfo?SN=NoNameServer?AdminPassword=abc?Password=abc?Mutator=XIIIMP.MarioMutator -port=7777");
        let mut k = 0;
        let m = admin_password(|| {
            k += 1;
            [5, 10, 35, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12][k - 1]
        });
        assert_eq!(m, "az0123456789abc");
    }
}

/// MSVCRT rand() (seed × 214013 + 2531011, bits 16 to 30), seeded from the clock.
fn rand_crt() -> impl FnMut() -> i32 {
    let mut seed_value = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_secs() as u32);
    move || {
        seed_value = seed_value.wrapping_mul(214013).wrapping_add(2531011);
        ((seed_value >> 16) & 0x7FFF) as i32
    }
}

/// oxiii launcher <game>: modes and maps; oxiii launcher <game> <mode> <map> [players=N] [ff] [points=N] [time=N]
/// [name=X] [port=N] [ubi=account:password[:private password]] [seconds=N] [log=path]: the ServerLauncher
/// line, then oxiii server.
pub fn launcher(game_value: &Path, args: &[String]) -> Result<(), String> {
    let list_value = maps_list(game_value);
    let Some(mode) = args.first() else {
        for (i, m) in MODES.iter().enumerate() {
            println!("{i} {}", m.0);
            for c in list_value.iter().filter(|c| map_mode(c, i)) {
                println!("    [{}] {}  ({})", c.players, c.readable, c.file_value);
            }
        }
        println!("points: {POINTS:?}; time (minutes): {TIME:?}; 0 = no limit");
        return Ok(());
    };
    let mode: usize = mode.parse().ok().filter(|&m| m < MODES.len()).ok_or("mode: 0 to 5")?;
    let name_map = args.get(1).ok_or("map missing")?;
    let map_value = list_value
        .iter()
        .find(|c| map_mode(c, mode) && (c.file_value.eq_ignore_ascii_case(name_map) || c.readable.eq_ignore_ascii_case(name_map) || c.file_value.to_ascii_lowercase().trim_end_matches(".unr") == name_map.to_ascii_lowercase()))
        .ok_or_else(|| format!("{name_map}: map not available for mode {}", MODES[mode].0))?
        .clone();
    // Option `key=value`.
    let opt = |k: &str| args[2..].iter().find_map(|a| a.strip_prefix(&format!("{k}=")).map(str::to_string));
    let integer = |k: &str, d: i32| opt(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    let players = integer("players", map_value.players);
    if !(2..=map_value.players.max(2)).contains(&players) {
        return Err(format!("players: 2 to {}", map_value.players));
    }
    let (points, time_value) = (integer("points", 0), integer("time", 0));
    if !POINTS.contains(&points) || !TIME.contains(&time_value) {
        return Err(format!("points in {POINTS:?}, time in {TIME:?}"));
    }
    let ubi = opt("ubi").map(|v| {
        let mut p = v.splitn(3, ':').map(str::to_string);
        (p.next().unwrap_or_default(), p.next().unwrap_or_default(), p.next())
    });
    let r = Settings {
        mode,
        players,
        shots_friends: args[2..].iter().any(|a| a == "ff"),
        points,
        time_value,
        name_value: opt("name").unwrap_or_default(),
        port: opt("port").unwrap_or_else(|| "7777".into()),
        ubi,
        map_value,
    };
    let password = admin_password(rand_crt());
    let line_value = line_value(&r, &password);
    println!("ucc {line_value}");
    println!("administration password: {password}");
    let mut words_list: Vec<String> = line_value.split(' ').map(str::to_string).collect();
    if let Some(s) = opt("seconds") {
        words_list.insert(1, s);
    }
    // OpenXIII-specific: log=path (UCC's ABSLOG=) so nothing is written to the game folder.
    if let Some(j) = opt("log") {
        words_list.push(format!("ABSLOG=\"{j}\""));
    }
    crate::server::server(game_value, &words_list)
}
