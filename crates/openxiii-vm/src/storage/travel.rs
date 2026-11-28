//! Travel from one map to another, reimplemented from Engine.dll: Unreal URLs (FURL, relative), server
//! travel (LevelInfo.NextURL after NextSwitchCountdown) and client travel (ClientTravel) handled by
//! UGameEngine::Tick, inventory exported by UGameEngine::ExportTravel
//! and taken back after Login by ULevel::SpawnPlayActor.

use crate::config::CPF_TRANSIENT;

/// Internal flag of export_text: Data and Delta at the same address (UProperty::ExportText
/// does not call Identical), everything is exported.
pub(crate) const EXPORT_ALL: u32 = 0x8000_0000;
use crate::types::{PropDef, Type};
use crate::value::{ClassId, ObjId, Value};
use crate::Vm;

// ETravelType.
pub const TRAVEL_ABSOLUTE: u8 = 0;
pub const TRAVEL_PARTIAL: u8 = 1;
pub const TRAVEL_RELATIVE: u8 = 2;

/// CPF_Travel.
const CPF_TRAVEL: u32 = 0x10000;

/// FURL: map, options ("Key=Value" or "Key"), portal ("#Teleporter2"), and for a
/// network address the host and the port ("192.0.2.2:7777").
#[derive(Debug, Clone, PartialEq)]
pub struct Url {
    pub map: String,
    pub options: Vec<String>,
    pub portal: String,
    /// Options added by `add_default_options` during `parse`, for the log.
    pub added_defaults: Vec<String>,
    /// Protocol (+0x00, None: DefaultProtocol "unreal"), Host (+0x0C, empty: DefaultHost) and
    /// Port (+0x18, None: DefaultPort 7777).
    pub protocol: Option<String>,
    pub host: String,
    pub port: Option<i32>,
    /// FURL::Valid (+0x40): 0 after a syntax error (default fields).
    pub valid: bool,
}

/// FURL defaults (FURL::StaticInit, [URL] of the ini): map DefaultMap "Index.unr"; default protocol,
/// host, port and portal (None, empty, None, empty).
impl Default for Url {
    fn default() -> Self {
        Url { map: "Index.unr".to_string(), options: Vec::new(), portal: String::new(), added_defaults: Vec::new(), protocol: None, host: String::new(), port: None, valid: true }
    }
}

/// appStrnicmp(a, b, n) == 0 (msvcr70 _strnicmp, ASCII case): the first n characters, the end
/// of a string counting as a null character.
pub(crate) fn strnicmp(a: &str, b: &str, n: usize) -> bool {
    let (mut x, mut y) = (a.chars(), b.chars());
    for _ in 0..n {
        match (x.next(), y.next()) {
            (None, None) => return true,
            (Some(c), Some(d)) if c.eq_ignore_ascii_case(&d) => {}
            _ => return false,
        }
    }
    true
}

/// FURL::DefaultPort without a configuration file ([URL] Port=7777 of XIII.ini).
pub const DEFAULT_PORT: i32 = 7777;

/// FURL defaults set by FURL::StaticInit from [URL] of the system file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UrlDefaults {
    pub protocol: String,
    pub protocol_description: String,
    pub name: String,
    pub map: String,
    pub local_map: String,
    pub host: String,
    pub portal: String,
    pub map_ext: String,
    pub save_ext: String,
    pub port: i32,
}

/// FURL::StaticInit: each key through GConfig->GetStr("URL", key, NULL) (GetString into
/// an appStaticString1024 buffer reset to empty: missing key = "", at most 1023 characters),
/// Port through appAtoi.
pub fn url_static_init(get: &mut dyn FnMut(&str) -> Option<String>) -> UrlDefaults {
    let mut s = |k: &str| get(k).map(|v| v.chars().take(1023).collect::<String>()).unwrap_or_default();
    UrlDefaults {
        protocol: s("Protocol"),
        protocol_description: s("ProtocolDescription"),
        name: s("Name"),
        map: s("Map"),
        local_map: s("LocalMap"),
        host: s("Host"),
        portal: s("Portal"),
        map_ext: s("MapExt"),
        save_ext: s("SaveExt"),
        port: crate::config::atoi(&s("Port").chars().collect::<Vec<_>>(), 0),
    }
}

impl crate::Vm {
    /// FURL::DefaultPort: url_static_init on the game's configuration (DEFAULT_PORT without a file).
    pub(crate) fn url_default_port(&mut self) -> i32 {
        match self.config.as_mut() {
            Some(c) => url_static_init(&mut |k| c.get_string("URL", k, "System")).port,
            None => DEFAULT_PORT,
        }
    }
}

impl Url {
    /// FURL::FURL(Base, TextURL, Type). Relative: the URL takes the protocol, the host,
    /// the port, the map and the portal of the base; partial or relative: its options, except PUSH,
    /// POP, PEER and LOAD (appStricmp of the whole option). Then the text: leading spaces
    /// skipped, "?option" and "#portal" segments (a space in a segment invalidates the URL), the head
    /// (drive "X:…": map only; "proto:" if the colon is not before a dot and not
    /// in the first two characters; "//host"; a dotted name that is neither .MapExt nor .SaveExt is
    /// "host[:port][/map]"), the base's default options (internal absolute travel), the map (with
    /// "/portal" for an internal URL), and finally the URL is invalid (default fields, Valid = 0) if
    /// the protocol, the host or the portal contain a space, or if nothing was given (neither host,
    /// nor internal map, nor drive, nor option).
    pub fn parse(base: Option<&Url>, text: &str, travel_type: u8) -> Url {
        let mut url = Url::default();
        if let Some(b) = base {
            if travel_type == TRAVEL_RELATIVE {
                url.protocol = b.protocol.clone();
                url.host = b.host.clone();
                url.port = b.port;
                url.map = b.map.clone();
                url.portal = b.portal.clone();
            }
            if travel_type == TRAVEL_RELATIVE || travel_type == TRAVEL_PARTIAL {
                for o in &b.options {
                    if !["PUSH", "POP", "PEER", "LOAD"].iter().any(|k| o.eq_ignore_ascii_case(k)) {
                        url.options.push(o.clone());
                    }
                }
            }
        }
        let text = text.trim_start_matches(' ');
        let (head, rest) = match text.find(['?', '#']) {
            Some(i) => (&text[..i], Some(&text[i..])),
            None => (text, None),
        };
        if let Some(rest) = rest {
            let mut kind = rest.as_bytes()[0];
            let mut s = &rest[1..];
            loop {
                let (seg, next) = match s.find(['?', '#']) {
                    Some(i) => (&s[..i], Some((s.as_bytes()[i], &s[i + 1..]))),
                    None => (s, None),
                };
                if seg.contains(' ') {
                    return Url::invalid();
                }
                if kind == b'?' {
                    url.add_option(seg);
                } else {
                    url.portal = seg.to_string();
                }
                match next {
                    Some((k, n)) => {
                        kind = k;
                        s = n;
                    }
                    None => break,
                }
            }
        }
        // Head: flags (ebp−0x20 and ebp−0x24) and map text (ebp−0x18).
        let (mut host_or_reader, mut map_internal) = (false, false);
        let mut text_map: Option<&str> = None;
        let b = head.as_bytes();
        if b.len() > 2 && b[1] == b':' {
            url.protocol = None;
            url.host.clear();
            url.map = head.to_string();
            url.portal.clear();
            host_or_reader = true;
            map_internal = true;
        } else {
            let mut s = head;
            if let Some(c) = s.find(':') {
                if c > 1 && s.find('.').is_none_or(|d| c < d) {
                    url.protocol = Some(s[..c].to_string());
                    s = &s[c + 1..];
                }
            }
            if s.starts_with('/') {
                match s[1..].strip_prefix('/') {
                    Some(r) => {
                        s = r;
                        url.host.clear();
                        host_or_reader = true;
                    }
                    None => return Url::invalid(),
                }
            }
            text_map = Some(s);
            if let Some(d) = s.find('.').filter(|&d| d > 0) {
                let ext = &s[d + 1..];
                let is_ext = |e: &str| ext.len() >= e.len() && ext.as_bytes()[..e.len()].eq_ignore_ascii_case(e.as_bytes()) && !ext.as_bytes().get(e.len()).is_some_and(|c| c.is_ascii_alphanumeric());
                if !is_ext("unr") && !is_ext("usa") {
                    let (host_port, map_value) = match s.find('/') {
                        Some(i) => (&s[..i], Some(&s[i + 1..])),
                        None => (s, None),
                    };
                    match host_port.find(':') {
                        Some(i) => {
                            url.port = Some(crate::config::atoi(&host_port[i + 1..].chars().collect::<Vec<_>>(), 0));
                            url.host = host_port[..i].to_string();
                        }
                        None => url.host = host_port.to_string(),
                    }
                    url.map = if url.is_internal() { "Index.unr".to_string() } else { String::new() };
                    host_or_reader = true;
                    text_map = map_value;
                }
            }
        }
        if let (TRAVEL_ABSOLUTE, Some(b), true) = (travel_type, base, url.is_internal()) {
            url.add_default_options(b);
        }
        if let Some(mt) = text_map.filter(|m| !m.is_empty()) {
            if url.is_internal() {
                map_internal = true;
                match mt.split_once('/') {
                    Some((map_value, after)) => {
                        match after.split_once('/') {
                            Some((portal_value, queue)) => {
                                if !queue.is_empty() {
                                    return Url::invalid();
                                }
                                url.portal = portal_value.to_string();
                            }
                            None => url.portal = after.to_string(),
                        }
                        url.map = map_value.to_string();
                    }
                    None => url.map = mt.to_string(),
                }
            } else {
                url.map = mt.to_string();
            }
        }
        let space = |s: &str| s.contains(' ');
        if space(url.protocol.as_deref().unwrap_or("unreal")) || space(&url.host) || space(&url.portal) || (!host_or_reader && !map_internal && url.options.is_empty()) {
            return Url::invalid();
        }
        url
    }

    /// FURL(NULL) copied after a syntax error: default fields, Valid = 0.
    fn invalid() -> Url {
        Url { valid: false, ..Url::default() }
    }

    /// FURL::IsInternal: default protocol.
    pub fn is_internal(&self) -> bool {
        self.protocol.as_deref().is_none_or(|p| p.eq_ignore_ascii_case("unreal"))
    }

    /// FURL::IsLocalInternal: internal and without a host.
    pub fn is_local_internal(&self) -> bool {
        self.is_internal() && self.host.is_empty()
    }

    /// FURL::FURL: absolute travel to an internal URL (local map) with
    /// a base, the base's options Name=, Team=, Class=, Skin=, Face=, Voice= and OverrideClass=
    /// that the URL does not have (empty GetOption) are added to it, in the base's order (the
    /// engine writes "URL: Adding default option …" in the log).
    fn add_default_options(&mut self, base: &Url) {
        const KEYS: [&str; 7] = ["Name=", "Team=", "Class=", "Skin=", "Face=", "Voice=", "OverrideClass="];
        for o in &base.options {
            if !KEYS.iter().any(|k| o.len() >= k.len() && o.as_bytes()[..k.len()].eq_ignore_ascii_case(k.as_bytes())) {
                continue;
            }
            // appStrncpy(key, option, n + 1) copies n characters: the key stops before the "=";
            // without "=", the whole option (appStrcpy).
            let key = match o.find('=') {
                Some(i) => &o[..i],
                None => o.as_str(),
            };
            if self.get_option(key).is_none_or(|v| v.is_empty()) {
                self.added_defaults.push(o.clone());
                self.options.push(o.clone());
            }
        }
    }

    /// FURL::LoadURLConfig: AddOption of each "Key=Value" line returned by GConfig->GetSection.
    pub fn load_url_config(&mut self, pairs: &[(String, String)]) {
        for (k, v) in pairs {
            self.add_option(&format!("{k}={v}"));
        }
    }

    /// FURL::SaveURLConfig: the GConfig->SetString(Section, key, value, Filename) calls it makes, in order: each
    /// "Key=Value" option (cut at the first "=") whose key is `item` ignoring case (appStricmp).
    pub fn save_url_config(&self, item: &str) -> Vec<(String, String)> {
        self.options.iter().filter_map(|o| o.split_once('=')).filter(|(k, _)| k.eq_ignore_ascii_case(item)).map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// FURL::AddOption: n = length of the key (up to the "=", otherwise the whole
    /// option); the first option whose first n + 1 characters equal those of `op` ignoring
    /// case (appStrnicmp: "=" included, or the end of the option) is replaced, otherwise added.
    pub fn add_option(&mut self, op: &str) {
        let n = op.chars().position(|c| c == '=').unwrap_or(op.chars().count());
        match self.options.iter_mut().find(|o| strnicmp(o, op, n + 1)) {
            Some(o) => *o = op.to_string(),
            None => self.options.push(op.to_string()),
        }
    }

    /// FURL::GetOption: the rest of the first option that starts with `key`
    /// (appStrnicmp on the length of `key`, nothing else is checked: an empty key takes the
    /// first whole option).
    pub fn get_option(&self, key: &str) -> Option<&str> {
        let n = key.chars().count();
        self.options.iter().find(|o| strnicmp(o, key, n)).map(|o| o.char_indices().nth(n).map_or("", |(i, _)| &o[i..]))
    }

    /// FURL::HasOption: an option equal to `key`, ignoring case (appStricmp).
    pub fn has_option(&self, key: &str) -> bool {
        self.options.iter().any(|o| strnicmp(o, key, usize::MAX))
    }

    /// FURL::String: "proto:" (and "//" with a host) outside the default protocol,
    /// "host[:port]/" with a host or a port other than 7777, map, "?options", "#portal".
    pub fn string(&self) -> String {
        let mut s = String::new();
        // Protocol != DefaultProtocol (FString !=, case-insensitive): "unreal:" given in full is not rewritten.
        if let Some(p) = self.protocol.as_ref().filter(|p| !p.eq_ignore_ascii_case("unreal")) {
            s.push_str(p);
            s.push(':');
            if !self.host.is_empty() {
                s.push_str("//");
            }
        }
        let port = self.port.filter(|&p| p != DEFAULT_PORT);
        if !self.host.is_empty() || port.is_some() {
            s.push_str(&self.host);
            if let Some(p) = port {
                s.push_str(&format!(":{p}"));
            }
            s.push('/');
        }
        s.push_str(&self.map);
        for o in &self.options {
            s.push('?');
            s.push_str(o);
        }
        if !self.portal.is_empty() {
            s.push('#');
            s.push_str(&self.portal);
        }
        s
    }

    /// "?Key=Value…" for GameInfo.Login.
    pub fn options_string(&self) -> String {
        self.options.iter().map(|o| format!("?{o}")).collect()
    }
}

/// Starting URL (UGameEngine::Init): the options of the default player ([DefaultPlayer] of
/// the player's ini, FURL::LoadURLConfig: AddOption of each "Key=Value" line), then the requested map ("Hual02", "Hual02#Portal").
pub fn start_url(system_dir: &std::path::Path, map: &str) -> Url {
    let mut config = crate::config::ConfigCache::new(system_dir.to_path_buf());
    let mut base = Url::default();
    base.load_url_config(&config.section_pairs("DefaultPlayer", "User"));
    Url::parse(Some(&base), map, TRAVEL_PARTIAL)
}

/// Map file of a URL (.unr extension added if needed).
pub fn map_file(url: &Url) -> String {
    if url.map.contains('.') {
        url.map.clone()
    } else {
        format!("{}.unr", url.map)
    }
}

/// Travel requested from the host (UGameEngine::Browse): URL to load and inventory of each player.
#[derive(Debug, Clone)]
pub struct TravelRequest {
    pub url: Url,
    /// TravelInfo: player name → ExportTravel text.
    pub info: Vec<(String, String)>,
    /// Engine subsystems (saves, music) taken over by the next map.
    pub engine: Option<Box<crate::save::EngineState>>,
    /// Map of a successful pending level (UGameEngine::Tick: LoadMap(Pending->URL)): the
    /// driver stays, and after loading the VM waits for `net_after_pending_load` (JOIN).
    pub pending_join: bool,
}

impl Vm {
    /// UDelegateProperty::ExportTextItem: "%s.%s", name of
    /// the object then name of the function (empty FName = NAME_None, written "None").
    pub(crate) fn delegate_export_text(obj: &str, func: &str) -> String {
        let func = if func.is_empty() { "None" } else { func };
        format!("{obj}.{func}")
    }

    /// UGameEngine::Tick after the level: server travel when NextSwitchCountdown
    /// reaches zero (inventory exported except for "?RESTART"), then client travel. Whole
    /// LoadMap, without a PaintProgress frame (windowless tools): `engine_travel_tick_start` then
    /// `engine_load_map_continued`.
    pub fn engine_travel_tick(&mut self, dt: f32) -> Option<TravelRequest> {
        let mut req = self.engine_travel_tick_start(dt);
        if let Some(r) = req.as_mut() {
            self.engine_load_map_continued(r);
        }
        req
    }

    /// UGameEngine::Tick up to LoadMap's PaintProgress: a returned travel has already done
    /// "Start LoadMap" and EndMap; the old level, its controller and its pawn are still there for
    /// the PaintProgress frame (the original game draws it with the player's view). The window draws
    /// this frame, then calls `engine_load_map_continued`.
    pub fn engine_travel_tick_start(&mut self, dt: f32) -> Option<TravelRequest> {
        // UInteractionMaster::MasterProcessTick after GLevel->Tick, with the real duration. Dedicated server: no
        // UClient (InteractionMaster at +0x88), no VideoPlayerDevice, no SaveGameDevice (created
        // by UGameEngine::Init only if GIsClient).
        let client = !self.ipdrv.dedicated;
        if client {
            self.master_tick(dt);
        }
        // Improved build: joining a co-op host waits for the character screen (character_page.rs).
        crate::improved::storage::character_screen_client_travel(self);
        let mut req = self.engine_travel_tick_inner(dt);
        // Not in the original: a client joining its server again (net_reconnect.rs).
        self.net_reconnect_tick();
        // UGameEngine::Tick: Client->Tick (rendering, skipped during a video), then
        // VideoPlayerDevice->Tick before SaveGameDevice->Tick.
        if client {
            self.video_tick(dt);
        }
        // SaveGameDevice->Tick at the end of the frame; loading a checkpoint travels last.
        if let Some(r) = if client { self.save_tick() } else { None } {
            req = Some(r);
        }
        // End of UGameEngine::Tick: network failure reported (ReportNetFailure) and delay elapsed:
        // SetClientTravel(Viewports(0), "?failed", 0, TRAVEL_Absolute), taken at the next frame.
        if self.ipdrv.connection_failure && self.ipdrv.time_to_wait_up_to <= self.app_seconds {
            // Not in the original: enhanced network play, a classic multiplayer client joins its
            // server again (net_reconnect.rs).
            match self.net_reconnect_rejoin() {
                Some(host) => {
                    self.ipdrv.connection_failure = false;
                    self.client_travel = Some((host, TRAVEL_ABSOLUTE, false));
                }
                None => self.client_travel = Some(("?failed".into(), TRAVEL_ABSOLUTE, false)),
            }
        }
        // Improved build: a co-op game's start waits for the character screen (character_page.rs).
        let req = crate::improved::storage::character_screen_travel(self, req);
        if let Some(r) = req.as_ref() {
            // UGameEngine::LoadMap: "Start LoadMap: %s" first, before the
            // actors of the old level's viewports are destroyed.
            self.audio_start_load_map();
            self.log.push(format!("Log: Start LoadMap: {}", r.url.string()));
            self.start_loadmap_logged = true;
            self.end_map_interactions();
        }
        req
    }

    /// Rest of UGameEngine::LoadMap after PaintProgress: actors of the old level's viewports
    /// destroyed (loop, after PaintProgress and the loading of the new
    /// map), network driver, GC log, then the engine handed over to the travel.
    pub fn engine_load_map_continued(&mut self, r: &mut TravelRequest) {
        {
            self.load_map_destroy_viewport_actors();
            // UGameEngine::LoadMap: the old level's NetDriver is destroyed (connections
            // closed, "NetComeGo: Close"); the map of a pending level keeps its own,
            // another travel abandons the pending level (CancelPending).
            // Packages of the host's MasterMap (UPackageMap::Serialize kept them alive): released with the driver, before GC1.
            let master = self.gc_master_packages();
            if !r.pending_join {
                self.engine_cancel_pending();
                self.net_destroy_driver();
            }
            // GC1 of UGameEngine::LoadMap when the map changes (log only, gc_log.rs).
            let strip = |m: &str| m.rsplit_once('.').map_or(m.to_string(), |(a, _)| a.to_string());
            let (old, new) = (strip(&self.url.map), strip(&r.url.map));
            if !old.eq_ignore_ascii_case(&new) {
                let held = self.gc_net_held_packages();
                self.gc_log_travel(&old, &new, &master, &held);
            }
        }
        {
            // UGameEngine::LoadMap: appStricmp(URL.Map, LastURL.Map), the first
            // CollectGarbage (all actors eliminated) only happens if the map changes.
            self.same_map_roots = r.url.map.eq_ignore_ascii_case(&self.url.map).then(|| self.level_gc_roots());
            let unique = self.class_unique_by_name();
            let kept = self.load_order.take().map(|lo| self.lo_prune_for_travel(lo));
            let mut e = std::mem::take(&mut self.engine);
            e.load_order = kept;
            e.config = self.config.clone();
            e.class_unique = unique;
            r.engine = Some(Box::new(e));
        }
    }

    /// Start of UGameEngine::LoadMap, when a level is loaded: each
    /// UInteraction object (TObjectIterator) receives EndMap (ProcessEvent), before PaintProgress.
    fn end_map_interactions(&mut self) {
        let Some(class) = self.class("Interaction") else { return };
        if self.level.is_none() {
            return;
        }
        let mut list: Vec<ObjId> = (0..self.objects.len())
            .filter(|&o| self.objects[o].as_ref().is_some_and(|x| self.is_child_of(x.class, class)) && !self.is_cdo(o))
            .collect();
        self.sort_original(&mut list);
        for o in list {
            if self.objects[o].is_none() {
                continue;
            }
            if let Some(f) = self.find_function(o, "EndMap", false) {
                if let Err(e) = self.call(f, o, vec![]) {
                    self.warn(format!("EndMap of {}: {e}", self.obj_label(o)));
                }
            }
        }
    }

    /// UGameEngine::LoadMap (before ResetLoaders): for each viewport of the client,
    /// its PlayerController loses its Player (+0x330) and the viewport its Actor; the controller's
    /// pawn is destroyed in the old level (XLevel->DestroyActor), then the controller.
    fn load_map_destroy_viewport_actors(&mut self) {
        for slot in 0..4u8 {
            let Some(viewport) = self.find_viewport_object(slot) else { continue };
            let Some(pc) = self.propr(viewport, "Actor").as_object().filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some())) else {
                continue;
            };
            self.set_prop(pc, "Player", Value::Object(None));
            self.set_prop(viewport, "Actor", Value::Object(None));
            if let Some(pawn) = self.propr(pc, "Pawn").as_object() {
                if let Err(e) = self.destroy_actor(pawn) {
                    self.warn(format!("LoadMap, destruction of {}: {e}", self.obj_label(pawn)));
                }
            }
            if let Err(e) = self.destroy_actor(pc) {
                self.warn(format!("LoadMap, destruction of {}: {e}", self.obj_label(pc)));
            }
        }
    }

    fn engine_travel_tick_inner(&mut self, dt: f32) -> Option<TravelRequest> {
        if let Some(level) = self.level {
            let next = self.propr(level, "NextURL").as_name();
            if !next.is_empty() {
                let c = self.propr(level, "NextSwitchCountdown").as_float() - dt;
                self.set_prop(level, "NextSwitchCountdown", Value::Float(c));
                if c <= 0.0 {
                    let items = !next.eq_ignore_ascii_case("?RESTART") && self.propr(level, "bNextItems").as_bool();
                    let info = if items { self.export_players_travel() } else { Vec::new() };
                    // UGameEngine::Tick: "Server switch level: %s" before Browse.
                    self.log.push(format!("Log: Server switch level: {next}"));
                    self.set_prop(level, "NextURL", Value::Str(String::new()));
                    return Some(self.browse_request(&next, TRAVEL_RELATIVE, info));
                }
            }
        }
        if let Some((url, travel_type, items)) = self.client_travel.take() {
            let info = if items { self.export_players_travel() } else { Vec::new() };
            let req = self.browse_request(&url, travel_type, info);
            let req = self.engine_browse_network(req);
            // UGameEngine::Tick (after Browse vtable +0xDC): successful Browse →
            // ConnectionFailure = 0. A local map (including "?failed" → LocalMap) is loaded by
            // the caller; without this reset, "?failed" started again every frame (menu
            // reloaded endlessly, keys ignored by UGameEngine::InputEvent).
            if req.is_some() {
                self.ipdrv.connection_failure = false;
            }
            return req;
        }
        // Pending level, with no client travel requested.
        if self.ipdrv.pending.is_some() && self.ipdrv.engine_pending {
            return self.engine_pending_tick(dt);
        }
        None
    }

    /// UGameEngine::Browse of a non-local internal URL: pending level
    /// (UNetPendingLevel: client driver, HELLO), after CancelPending; a driver
    /// failure: SetProgress("Networking Failed", error, 6) and ReportNetFailure(4, 1, error).
    /// A URL with another protocol (appLaunchURL) is not followed. Without `engine_pending` (command-line
    /// tools), the travel is returned as is.
    fn engine_browse_network(&mut self, req: TravelRequest) -> Option<TravelRequest> {
        // Lockstep co-op (lockstep.rs): joining a co-op host is the window's (it meets the host
        // and loads the host's start map itself).
        if crate::lockstep::wanted() && !req.url.host.is_empty() && req.url.has_option(crate::coop_address::COOP_OPTION) {
            return Some(req);
        }
        if !self.ipdrv.engine_pending || req.url.is_local_internal() {
            // Not in the original: a map of this machine, no server to join again.
            if req.url.is_local_internal() {
                self.net_reconnect_local_travel();
            }
            return Some(req);
        }
        if !req.url.is_internal() {
            self.log.push(format!("Log: appLaunchURL {}", req.url.string()));
            return None;
        }
        self.engine_cancel_pending();
        self.ipdrv.connection_status = 0;
        self.ipdrv.connection_error.clear();
        self.net_reconnect_note_join(&req.url);
        if let Err(e) = self.net_connect(&req.url.string()) {
            self.engine_set_progress("Networking Failed", &e, 6.0);
            self.engine_report_net_status(&e);
            self.ipdrv.pending = None;
            self.ipdrv.net_driver = None;
        }
        None
    }

    /// UGameEngine::Tick: Tick of the pending level; an error:
    /// SetProgress(ConnectionFailed, error, 4), ReportNetFailure(4, 1, error), log
    /// "Pending connect to '…' failed; …", level abandoned; a WELCOME without a file to
    /// download: LoadMap(Pending->URL), then JOIN (`net_after_pending_load`).
    pub(crate) fn engine_pending_tick(&mut self, dt: f32) -> Option<TravelRequest> {
        self.net_pending_tick(dt);
        let p = self.ipdrv.pending.as_ref()?;
        if !p.error.is_empty() {
            let (err, url) = (p.error.clone(), p.url.clone());
            let failed = self.localize("Errors", "ConnectionFailed", "Engine", false);
            self.engine_set_progress(&failed, &err, 4.0);
            self.engine_report_net_status(&err);
            let fmt = self.localize("Errors", "Pending", "Engine", false);
            let line = fmt.replacen("%s", &url, 1).replacen("%s", &err, 1);
            self.log.push(format!("Log: {line}"));
            self.net_destroy_driver();
            // Not in the original: while joining the server again, another attempt.
            self.net_reconnect_pending_failed();
            return None;
        }
        if !(p.success && p.files_needed == 0 && !p.sent_join) {
            return None;
        }
        let mut url = Url::parse(None, &p.url, TRAVEL_ABSOLUTE);
        url.map = p.level.clone();
        if let Some(p) = self.ipdrv.pending.as_mut() {
            p.sent_join = true;
        }
        self.engine_gui_cleanup();
        Some(TravelRequest { url, info: Vec::new(), engine: None, pending_join: true })
    }

    /// UGameEngine::GUICleanup: each BaseGUIController of the LocalInteractions of the
    /// viewports of Client->Viewports (in array order) receives CloseAll(bCancel true).
    pub(crate) fn engine_gui_cleanup(&mut self) {
        for slot in self.engine.client.viewports.clone() {
            let Some(v) = self.find_viewport_object(slot) else { continue };
            let list = match self.propr(v, "LocalInteractions") {
                Value::Array(a) => a.iter().filter_map(|x| x.as_object()).collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            for o in list {
                if self.alive(o) && self.is_a(o, "BaseGUIController") {
                    self.event_pub(o, "CloseAll", vec![Value::Bool(true)]);
                }
            }
        }
    }

    /// UGameEngine::CancelPending: control channel of the pending level closed,
    /// FlushNet, then the level (and its driver) destroyed.
    pub(crate) fn engine_cancel_pending(&mut self) {
        if self.ipdrv.pending.is_none() {
            return;
        }
        self.net_close_server_connection();
        self.net_destroy_driver();
        self.ipdrv.pending = None;
    }

    /// End of loading of a pending level's map (UGameEngine::Tick): except for a
    /// lone game (LONE), SendJoin; the pending level disappears.
    pub fn net_after_pending_load(&mut self) {
        // Not in the original: joined again (net_reconnect.rs).
        self.net_reconnect_joined();
        let lone = self.ipdrv.pending.as_ref().is_some_and(|p| p.lone);
        if lone {
            self.ipdrv.pending = None;
            return;
        }
        self.net_join();
    }

    /// End of UGameEngine::LoadMap on a client: FURL::SaveURLConfig of Name, Team,
    /// Class, Skin, Face, Voice and OverrideClass in [DefaultPlayer] of User.ini (each
    /// "Key=Value" option with that key, ignoring case).
    pub fn load_map_save_url_config(&mut self) {
        for item in ["Name", "Team", "Class", "Skin", "Face", "Voice", "OverrideClass"] {
            for (k, v) in self.url.save_url_config(item) {
                if let Some(c) = self.config.as_mut() {
                    c.set_string("DefaultPlayer", &k, &v, "User");
                }
            }
        }
    }

    /// UGameEngine::Browse: URL relative to the current map; "?restart" reloads
    /// the current map.
    pub(crate) fn browse_request_pub(&mut self, text: &str, travel_type: u8, info: Vec<(String, String)>) -> TravelRequest {
        self.browse_request(text, travel_type, info)
    }

    fn browse_request(&mut self, text: &str, travel_type: u8, info: Vec<(String, String)>) -> TravelRequest {
        let mut url = Url::parse(Some(&self.url), text, travel_type);
        for o in std::mem::take(&mut url.added_defaults) {
            self.log.push(format!("Log: URL: Adding default option {o}"));
        }
        if url.has_option("restart") {
            url = self.url.clone();
        }
        // Not in the original: the menus host a network co-op game (Co-op page): its first map
        // is marked for the window's lockstep session (coop_address.rs).
        self.coop_lan_host_url(&mut url);
        // FURL: default map ([URL] Map) when the absolute URL gives none.
        if url.map.is_empty() && travel_type == TRAVEL_ABSOLUTE {
            url.map = self.config.as_mut().and_then(|c| c.get_string("URL", "Map", "System")).unwrap_or_default();
        }
        self.log.push(format!("Log: Browse: {}", url.string()));
        // "?failed": Browse(FURL(LastURL, DefaultLocalMap, TRAVEL_Absolute)).
        if url.has_option("failed") {
            let local = self.config.as_mut().and_then(|c| c.get_string("URL", "LocalMap", "System")).unwrap_or_default();
            return self.browse_request(&local, TRAVEL_ABSOLUTE, info);
        }
        TravelRequest { url, info, engine: None, pending_join: false }
    }

    /// Local players (PlayerController with a Player and a Pawn): their pawn then its inventory.
    pub(crate) fn export_players_travel(&mut self) -> Vec<(String, String)> {
        let pcs: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.is_a(a, "PlayerController")).collect();
        let mut out = Vec::new();
        // Co-op (coop.rs): each player's own text, by rank, for the next map's logins.
        let mut carried: Vec<Option<String>> = vec![None; self.coop.players.len()];
        for pc in pcs {
            let (Some(_), Some(pawn)) = (self.propr(pc, "Player").as_object(), self.propr(pc, "Pawn").as_object()) else {
                continue;
            };
            let mut text = String::new();
            self.export_travel(pawn, &mut text);
            let mut inv = self.propr(pawn, "Inventory").as_object();
            let mut guard = 0;
            while let (Some(i), true) = (inv, guard < 256) {
                self.export_travel(i, &mut text);
                inv = self.propr(i, "Inventory").as_object();
                guard += 1;
            }
            if let Some(rank) = self.coop.players.iter().position(|&p| p == pc) {
                carried[rank] = Some(text.clone());
            }
            let name = self.propr(pc, "PlayerReplicationInfo").as_object().map(|r| self.propr(r, "PlayerName").as_name()).unwrap_or_default();
            out.push((name, text));
        }
        self.coop.carried = if self.coop.on { carried } else { Vec::new() };
        out
    }

    /// UGameEngine::ExportTravel: "Class=… Name=…", then each travel property
    /// different from the class's default values (an actor by its name).
    pub(crate) fn export_travel(&mut self, a: ObjId, out: &mut String) {
        // UProperty::ExportText: skipped if Identical to the default value.
        self.export_travel_with(a, out, &mut |vm, p, _k, v, d| {
            let identical = match d {
                Some(d) => crate::identical(v, d),
                None => false,
            };
            (!identical).then(|| vm.export_text(&p.ty, p.enumeration.as_deref(), v, d, 0))
        });
    }

    /// ExportTravel with the text of each element given by `exporter` (property, rank of the element, value, default value):
    /// `None` when UProperty::ExportText does not write it.
    pub(crate) fn export_travel_with(&mut self, a: ObjId, out: &mut String, exporter: &mut dyn FnMut(&mut Vm, &PropDef, usize, &Value, Option<&Value>) -> Option<String>) {
        let Some((class, name)) = self.objects[a].as_ref().map(|o| (o.class, o.name.clone())) else {
            return;
        };
        self.log.push(format!("Log: Exporting travelling actor of class {}", self.class_path(class)));
        if !self.propr(a, "bTravel").as_bool() {
            return;
        }
        out.push_str(&format!("Class={} Name={name}\r\n{{\r\n", self.class_path(class)));
        let props = self.classes[class].props.clone();
        let cdo = self.classes[class].cdo;
        let values = self.objects[a].as_ref().map(|o| o.props.clone()).unwrap_or_default();
        let defaults = self.objects.get(cdo).and_then(|o| o.as_ref()).map(|o| o.props.clone()).unwrap_or_default();
        // TFieldIterator<UProperty>: the class, then its parents.
        for i in self.field_order(class) {
            let p = &props[i];
            if p.flags & CPF_TRAVEL == 0 {
                continue;
            }
            for k in 0..p.dim {
                let (v, d) = (element(values.get(i), p.dim, k), element(defaults.get(i), p.dim, k));
                let Some(v) = v else { continue };
                let Some(text) = exporter(self, p, k, v, d) else { continue };
                let key = if p.dim == 1 { p.name.clone() } else { format!("{}[{k}]", p.name) };
                // Object property of an actor class: "Name=<actor's name or None>", then the
                // ExportText line too.
                let actor_class = match &p.ty {
                    Type::Object(cls) => {
                        let actor = self.class("Actor");
                        self.class(cls.rsplit('.').next().unwrap_or(cls)).zip(actor).is_some_and(|(c, a)| self.is_child_of(c, a))
                    }
                    _ => false,
                };
                out.push_str(&format!("{key}="));
                if actor_class {
                    let o = v.as_object();
                    let name = o.and_then(|o| self.objects[o].as_ref().map(|x| x.name.clone())).unwrap_or_else(|| "None".into());
                    out.push_str(&format!("{name}\r\n"));
                }
                out.push_str(&format!("{text}\r\n"));
            }
        }
        out.push_str("}\r\n");
    }

    /// UProperty::ExportTextItem: text of a value (the opposite of ImportText). `default`: the
    /// comparison value (Delta); `flags`: PPF_Localized (1), PPF_Delimited (2), and
    /// EXPORT_ALL when Data and Delta are the same address (nothing is skipped).
    pub(crate) fn export_text(&mut self, ty: &Type, enumeration: Option<&str>, v: &Value, default: Option<&Value>, flags: u32) -> String {
        const PPF_LOCALIZED: u32 = 1;
        const PPF_DELIMITED: u32 = 2;
        match (ty, v) {
            // UIntProperty::ExportTextItem: "%i".
            (Type::Int, v) => v.as_int().to_string(),
            // UFloatProperty::ExportTextItem: appSprintf("%f").
            (Type::Float, v) => crate::value::printf_f(v.as_float()),
            // UBoolProperty::ExportTextItem: GTrue / GFalse with
            // PPF_Localized, otherwise "True" / "False".
            (Type::Bool, v) => {
                let t = if flags & PPF_LOCALIZED != 0 {
                    self.config.as_mut().and_then(|c| c.localize2("General", if v.as_bool() { "True" } else { "False" }, "Core"))
                } else {
                    None
                };
                t.unwrap_or_else(|| if v.as_bool() { "True".into() } else { "False".into() })
            }
            // UByteProperty::ExportTextItem: "%i" without an enum, otherwise the name
            // of the value, "(INVALID)" beyond the last one.
            (Type::Byte, v) => {
                let b = v.as_byte();
                match enumeration.and_then(|e| self.enum_names(e)) {
                    Some(names) if (b as usize) < names.len() => names[b as usize].clone(),
                    Some(_) => "(INVALID)".into(),
                    None => b.to_string(),
                }
            }
            // UNameProperty::ExportTextItem: *FName, in quotes with
            // PPF_Delimited.
            (Type::Name, v) => {
                let n = v.as_name();
                let n = if n.is_empty() { "None".to_string() } else { n };
                if flags & PPF_DELIMITED != 0 {
                    format!("\"{n}\"")
                } else {
                    n
                }
            }
            // UStrProperty::ExportTextItem: the string, or it in quotes ("%s") with
            // PPF_Delimited (without escaping).
            (Type::Str, v) => {
                let s = v.as_name();
                if flags & PPF_DELIMITED != 0 {
                    format!("\"{s}\"")
                } else {
                    s
                }
            }
            // UObjectProperty::ExportTextItem: "%s'%s'", name of the object's class
            // and GetPathName; "None".
            (Type::Object(_), Value::Object(Some(o))) => match self.objects[*o].as_ref().map(|x| self.classes[x.class].name.clone()) {
                Some(c) => format!("{c}'{}'", self.path_name(*o)),
                None => "None".into(),
            },
            (Type::Object(cls), Value::Asset(path)) => format!("{}'{path}'", cls.rsplit('.').next().unwrap_or(cls)),
            (Type::Class(_), Value::Class(Some(c))) => format!("Class'{}'", self.class_path(*c)),
            (Type::Object(_) | Type::Class(_), _) => "None".into(),
            // UDelegateProperty::ExportTextItem: "%s.%s", FName of the object and name
            // of the function (the original does not test for a null object).
            (Type::Delegate, Value::Delegate(o, f)) => {
                let obj = o.and_then(|o| self.objects.get(o).and_then(|x| x.as_ref()).map(|x| x.name.clone())).unwrap_or_else(|| "None".into());
                Self::delegate_export_text(&obj, f)
            }
            // UStructProperty::ExportTextItem: "(Field=…,Field[i]=…)", each
            // element through UProperty::ExportText (PPF_Delimited only): skipped if it is
            // Identical to Delta's (or to zero without Delta). The field named "Material"
            // goes first when the name exists.
            // (Inline vector / rotator: their fields seen as those of the struct.)
            (Type::Struct(name), v) if v.is_struct() => {
                let (_, fields) = v.struct_view().expect("struct");
                let defs = self.struct_def(name).map(|s| s.fields.clone()).unwrap_or_default();
                let zero = self.zero(ty, 1);
                let default_value = default.cloned().unwrap_or(zero);
                let default_fields: Vec<Value> = default_value.struct_view().map(|(_, f)| f.to_vec()).unwrap_or_default();
                let mut order: Vec<usize> = (0..defs.len()).collect();
                if let Some(m) = defs.iter().position(|f| f.name.eq_ignore_ascii_case("Material")) {
                    order.remove(m);
                    order.insert(0, m);
                }
                let mut parts = Vec::new();
                for fi in order {
                    let f = &defs[fi];
                    if f.name.eq_ignore_ascii_case("Class") || f.flags & (CPF_TRANSIENT | crate::config::CPF_NATIVE) != 0 {
                        continue;
                    }
                    // PPF 8 (localized values): only the string, struct and array fields go through (UStructProperty::ExportTextItem).
                    if flags & 8 != 0 && !matches!(f.ty, Type::Str | Type::Struct(_) | Type::Array(_)) {
                        continue;
                    }
                    for k in 0..f.dim {
                        let (fv, fd) = (element(fields.get(fi), f.dim, k), element(default_fields.get(fi), f.dim, k));
                        let Some(fv) = fv else { continue };
                        if flags & EXPORT_ALL == 0 && fd.is_some_and(|d| crate::identical(fv, d)) {
                            continue;
                        }
                        let text = self.export_text(&f.ty, f.enumeration.as_deref(), fv, fd, PPF_DELIMITED | (flags & EXPORT_ALL));
                        if f.dim == 1 {
                            parts.push(format!("{}={text}", f.name));
                        } else {
                            parts.push(format!("{}[{k}]={text}", f.name));
                        }
                    }
                }
                format!("({})", parts.join(","))
            }
            // UArrayProperty::ExportTextItem: "(a,b,…)", each element compared
            // with the one of the same rank in Delta, flags | PPF_Delimited.
            (Type::Array(inner), Value::Array(items)) => {
                let defaults = match default {
                    Some(Value::Array(d)) => d.clone(),
                    _ => Vec::new(),
                };
                let texts: Vec<String> = items.iter().enumerate().map(|(k, it)| self.export_text(inner, enumeration, it, defaults.get(k), flags | PPF_DELIMITED)).collect();
                format!("({})", texts.join(","))
            }
            _ => String::new(),
        }
    }

    /// ULevel::SpawnPlayActor, after Login: the player's travel inventory (their
    /// pawn takes the first block, the other classes are created at its place), the imported
    /// properties (actors by their name), then TravelPreAccept, AcceptInventory, TravelPostAccept.
    pub fn accept_travel(&mut self, game: ObjId, pc: ObjId, info: Option<&str>) {
        let Some(pawn) = self.propr(pc, "Pawn").as_object() else {
            return;
        };
        self.event_pub(pc, "TravelPreAccept", vec![]);
        self.event_pub(pawn, "TravelPreAccept", vec![]);
        // Blocks "Class=… Name=…", "{", lines up to "}": Parse looks for CLASS= then NAME= in the remaining text,
        // ParseLine (Exact, at most 255 characters) reads the block's line, the opening brace then each line.
        let mut entries: Vec<(ObjId, String, Vec<String>)> = Vec::new();
        let chars: Vec<char> = info.unwrap_or("").chars().collect();
        let mut pos = 0usize;
        while info.is_some() {
            let rest: String = chars[pos..].iter().collect();
            let Some(class) = crate::value::parse_fstring(&rest, "CLASS=").map(|v| v.chars().take(255).collect::<String>()) else { break };
            let Some(name) = crate::value::parse_fstring(&rest, "NAME=").map(|v| v.chars().take(255).collect::<String>()) else { break };
            self.log.push(format!("Log: Incoming travelling actor of class {class}"));
            let actor = match self.class(&class) {
                Some(c) if self.class("Pawn").is_some_and(|p| self.is_child_of(c, p)) => Some(pawn),
                Some(c) => {
                    let (l, r) = (self.prop(pawn, "Location"), self.prop(pawn, "Rotation"));
                    let test_value: Option<Value> = None;
                    let a = match test_value {
                        Some(v) => v.as_object(),
                        None => self.spawn_actor(c, Value::Object(Some(pawn)), None, l, r, Value::Object(None), true).ok().flatten(),
                    };
                    match a {
                        Some(_) => self.log.push("Log: SpawnPlayActor: Spawned travelling actor".into()),
                        None => self.log.push(format!("Log: SpawnPlayActor: Failed to spawn travelling class '{class}'")),
                    }
                    a
                }
                None => {
                    self.log.push(format!("Log: SpawnPlayActor: Cannot accept travelling class '{class}'"));
                    None
                }
            };
            let mut props = Vec::new();
            let _ = crate::player_input::parse_line_exact(&chars, &mut pos, 0x100);
            let _ = crate::player_input::parse_line_exact(&chars, &mut pos, 0x100);
            while let Some(line) = crate::player_input::parse_line_exact(&chars, &mut pos, 0x100) {
                let line: String = line.into_iter().collect();
                if line == "}" {
                    break;
                }
                props.push(line);
            }
            if let Some(a) = actor {
                entries.push((a, name, props));
            }
        }
        // Properties: each line "Name[index]=value", each property of the actor's class in
        // TFieldIterator order with the same name, travel and an index within its dimension.
        for e in 0..entries.len() {
            let (a, _, props) = entries[e].clone();
            let class = self.objects[a].as_ref().map(|o| o.class).unwrap();
            let defs = self.classes[class].props.clone();
            let order_value = self.field_order(class);
            for line in props {
                let line = line.trim_start_matches(' ');
                let name_end = line.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(line.len());
                let pname = &line[..name_end];
                let mut rest = &line[name_end..];
                let mut index = 0usize;
                if let Some(r) = rest.strip_prefix('[') {
                    let digits = r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len());
                    index = r[..digits].parse().unwrap_or(0);
                    if !r[digits..].starts_with(']') {
                        continue;
                    }
                    rest = &r[digits + 1..];
                }
                let Some(value) = rest.strip_prefix('=') else { continue };
                for &slot in &order_value {
                    let p = defs[slot].clone();
                    if p.flags & CPF_TRAVEL == 0 || !p.name.eq_ignore_ascii_case(pname) || index >= p.dim {
                        continue;
                    }
                    let target_actor =
                        matches!(&p.ty, Type::Object(c) if self.class(c).is_some_and(|c| self.class("Actor").is_some_and(|a| self.is_child_of(c, a))));
                    if target_actor {
                        // FString == (appStricmp): the first block with this name gives its actor.
                        if let Some((other, _, _)) = entries.iter().find(|(_, n, _)| n.eq_ignore_ascii_case(value)) {
                            let other = *other;
                            let mut props = std::mem::take(&mut self.objects[a].as_mut().unwrap().props);
                            if let Some(v) = element_mut(&mut props[slot], p.dim, index) {
                                *v = Value::Object(Some(other));
                            }
                            if let Some(o) = self.objects[a].as_mut() {
                                o.props = props;
                            }
                        }
                    } else {
                        let mut props = std::mem::take(&mut self.objects[a].as_mut().unwrap().props);
                        if let Some(v) = element_mut(&mut props[slot], p.dim, index) {
                            let chars: Vec<char> = value.chars().collect();
                            self.import_text_named(&p.name, &p.ty, p.enumeration.as_deref(), &chars, 0, 0, v);
                        }
                        if let Some(o) = self.objects[a].as_mut() {
                            o.props = props;
                        }
                    }
                }
            }
        }
        for e in (0..entries.len()).rev() {
            self.event_pub(entries[e].0, "TravelPreAccept", vec![]);
        }
        self.event_pub(game, "AcceptInventory", vec![Value::Object(Some(pawn))]);
        for e in (0..entries.len()).rev() {
            self.event_pub(entries[e].0, "TravelPostAccept", vec![]);
        }
        self.event_pub(pc, "TravelPostAccept", vec![]);
    }
}

/// Core's Parse(Text, "KEY=", FString): value::parse_fstring.
fn parse_value(stream: &str, key: &str) -> Option<String> {
    crate::value::parse_fstring(stream, key)
}

fn element(v: Option<&Value>, dim: usize, k: usize) -> Option<&Value> {
    let v = v?;
    if dim <= 1 {
        return (k == 0).then_some(v);
    }
    match v {
        Value::Array(items) => items.get(k),
        _ => None,
    }
}

fn element_mut(v: &mut Value, dim: usize, k: usize) -> Option<&mut Value> {
    if dim <= 1 {
        return (k == 0).then_some(v);
    }
    match v {
        Value::Array(items) => items.get_mut(k),
        _ => None,
    }
}

impl Vm {
    /// UGameEngine::LoadMap (from ResetLoaders to CollectGarbage): the level left is
    /// removed from the VM, its objects (the map's package: actors, objects they created) as well as its
    /// state; what UGameEngine::Init created once (InteractionMaster, console, GUIController,
    /// windows and viewports, Transient objects), the classes, their default values, GConfig,
    /// the random seed and the resource caches stay for the next map.
    pub fn unload_level(&mut self) {
        self.lockstep_interp_clear();
        self.first_render.clear();
        self.light_selections.clear();
        // CollectGarbage(RF_Native) of UGameEngine::LoadMap: the classes that nothing reaches any more
        // will be reloaded, ClassUnique restarts from 0 (class_gc.rs).
        let (alive, reached) = self.travel_gc();
        // Menu controls and pages that nothing reaches any more: destroyed by the collection, their name
        // is free (MakeUniqueObjectName, looks for the first free name).
        if let Some(g) = self.class("GUIComponent") {
            let map = self.map_package.map(|p| self.packages[p].name.to_lowercase());
            let paths_gui: std::collections::HashSet<String> =
                self.objects.iter().flatten().filter(|o| self.is_child_of(o.class, g)).map(|o| format!("{}.{}", o.outer, o.name).to_lowercase()).collect();
            let dead_value: Vec<ObjId> = (0..self.objects.len())
                .filter(|&o| {
                    !reached.contains(&o)
                        && self.objects[o].as_ref().is_some_and(|x| {
                            (self.is_child_of(x.class, g) || paths_gui.contains(&x.outer.to_lowercase()))
                                && self.classes[x.class].cdo != o
                                && !map.as_ref().is_some_and(|m| x.outer.split('.').next().is_some_and(|p| p.eq_ignore_ascii_case(m)))
                        })
                })
                .collect();
            for o in dead_value {
                if let Some(x) = self.objects[o].take() {
                    self.object_paths.remove(&format!("{}.{}", x.outer, x.name).to_lowercase());
                }
            }
        }
        self.same_map_roots = None;
        self.class_unique.retain(|c, _| alive.contains(c));
        // The destroyed classes will be reloaded from their package: serialized default values
        // (UClass::Serialize), parent class first. Not those of the map's package, which goes
        // away with it.
        let map_pkg = self.map_package;
        let depth = |vm: &Vm, mut c: ClassId| {
            let mut d = 0;
            while let Some(s) = vm.classes[c].super_class {
                c = s;
                d += 1;
            }
            d
        };
        let mut dead: Vec<ClassId> = (0..self.classes.len()).filter(|c| !alive.contains(c) && Some(self.classes[*c].package) != map_pkg).collect();
        dead.sort_by_key(|&c| depth(self, c));
        let trace = false;
        for c in dead {
            // The sub-objects of the class's default values (Outer = the class: ParticleEmitters
            // of the Emitters, controls of GUI pages…) go away with it at CollectGarbage; on
            // reload, ULinkerLoad recreates them from their serialized properties.
            let prefix = format!("{}.{}.", self.packages[self.classes[c].package].name, self.classes[c].name).to_lowercase();
            let stale: Vec<String> = self.subobjects.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
            for k in stale {
                if let Some(id) = self.subobjects.remove(&k) {
                    if let Some(o) = self.objects.get(id).and_then(|o| o.as_ref()) {
                        let path = format!("{}.{}", o.outer, o.name).to_lowercase();
                        self.object_paths.remove(&path);
                    }
                }
            }
            let changed = self.reload_class_defaults(c);
            if trace && !changed.is_empty() {
                self.log.push(format!("TRACE class_gc: {} reloaded, defaults restored: {}", self.classes[c].name, changed.join(", ")));
            }
        }
        let dead: Vec<String> = self.engine.class_unique.keys().filter(|n| !self.class(n).is_some_and(|c| alive.contains(&c))).cloned().collect();
        for n in dead {
            self.engine.class_unique.remove(&n);
        }
        let map = self
            .map_package
            .and_then(|pi| self.packages.get(pi).map(|p| p.name.clone()))
            .or_else(|| self.map_file.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default()
            .to_lowercase();
        // The package of the map left keeps its place (package indices do not move)
        // but is no longer found by its name: the same map can be reloaded. Its bytes
        // are released (nothing reads its exports any more); its tables stay.
        if let Some(pi) = self.map_package {
            if let Some(p) = self.packages.get_mut(pi) {
                p.name = format!("\u{1}{}", p.name);
                p.data = Vec::new();
            }
        }
        // Network cache path → object of the map's objects (net_rep.rs): rebuilt for the next one.
        self.ipdrv.rep.map_objects = None;
        let prefix = format!("{map}.");
        let mut removed = std::collections::HashSet::new();
        if !map.is_empty() {
            for id in 0..self.objects.len() {
                let Some(o) = self.objects[id].as_ref() else { continue };
                let outer = o.outer.to_lowercase();
                if outer == map || outer.starts_with(&prefix) {
                    self.udp_link_gc_destroy(id);
                    let Some(o) = self.objects[id].as_ref() else { continue };
                    let path = format!("{}.{}", o.outer, o.name).to_lowercase();
                    self.object_paths.remove(&path);
                    self.objects[id] = None;
                    removed.insert(id);
                }
            }
        }
        let keep = |id: &ObjId| !removed.contains(id);
        self.subobjects.retain(|_, v| keep(v));
        self.threads.retain(|k, _| keep(k));
        self.probe_disabled.retain(|k, _| keep(k));
        self.state_changed.retain(keep);
        self.in_end_state.retain(keep);
        self.mesh_insts.retain(|k, _| keep(k));
        self.render_views.retain(|k, _| keep(k));
        if self.canvas.is_some_and(|c| removed.contains(&c)) {
            self.canvas = None;
        }
        // Level state: that of a new VM.
        let fresh = Vm::new();
        self.name_table = None;
        self.map_file = None;
        self.map_package = None;
        self.actors = fresh.actors;
        self.actors_changed();
        self.first_dynamic = 0;
        self.time = 0.0;
        self.pending_latent = None;
        self.in_state_code = false;
        self.state_code_obj = None;
        self.level = None;
        self.collision = None;
        // collision_materials is kept: the collision triangles of the StaticMeshes (sm_local, read
        // once per mesh) carry its indices from one map to the next.
        self.movers = fresh.movers;
        self.nav = fresh.nav;
        self.current_paths = fresh.current_paths;
        self.reach_xy_margin = 0.0;
        self.reach_height_margin = 0.0;
        self.set_path_pending = [None; 4];
        self.brushes = fresh.brushes;
        self.water_planes = fresh.water_planes;
        self.brush_boxes = fresh.brush_boxes;
        self.brush_box_keys = fresh.brush_box_keys;
        self.level_bsp = None;
        self.terrains = fresh.terrains;
        self.sm_shapes = fresh.sm_shapes;
        self.sm_generations = fresh.sm_generations;
        self.bsp_material_tags = fresh.bsp_material_tags;
        self.zone_actors = fresh.zone_actors;
        self.bsp_views = fresh.bsp_views;
        self.display = fresh.display;
        self.dyn_lights = fresh.dyn_lights;
        self.dyn_light_list = fresh.dyn_light_list;
        self.surf_actors = fresh.surf_actors;
        self.brush_polys = fresh.brush_polys;
        self.rendered = fresh.rendered;
        self.render_root = fresh.render_root;
        self.render_place = fresh.render_place;
        self.canvas_rendered = fresh.canvas_rendered;
        self.client_travel = None;
        self.default_volume = None;
        self.level_time = 0.0;
        self.in_tick = false;
        self.newly_spawned = fresh.newly_spawned;
        self.spawn_order = fresh.spawn_order;
        self.level_ticked = false;
        self.first_deleted = fresh.first_deleted;
        // Deleted actors of the map left: their package is released like the rest of the map.
        self.freed = fresh.freed;
        self.last_trace_bone = String::new();
        self.actor_removals = fresh.actor_removals;
        self.skel_render_boxes = fresh.skel_render_boxes;
        self.trails = fresh.trails;
        self.octree = fresh.octree;
        self.hud = fresh.hud;
        self.sounds = fresh.sounds;
        // The audio subsystem survives the travel (UGameEngine keeps Audio): CleanUp and
        // LoadSounds of the new map take care of it.
        self.audio_sounds = fresh.audio_sounds;
        self.index_sounds = Default::default();
        self.projectors = fresh.projectors;
        self.render_player = None;
        self.solo = fresh.solo;
        self.screen_fx = fresh.screen_fx;
        // Weather: the interface starts from zero, D3DDrv's statics (gusts, wind, strength) stay
        // (see `RndCubeSpr::kept_static`).
        let prior = std::mem::replace(&mut self.weather, fresh.weather);
        self.weather.kept_static(&prior);
        self.particles = fresh.particles;
        // First frame of the new map: precache frame (LevelAction 4) then full frame,
        // as at startup. The original does this at every load (two PostRenders: CanDisplay set by
        // the first, HudCartoonWnd created by the second). XIIIBaseHud.DrawCartoonWindowBis's
        // Rand(3) thus falls in frame 1, in the full frame, and the first DrawCartoonWindowBis no longer reads
        // an empty HudCartoonWnd ("DrawCartoonWindowBis:268B accessed none", E8). The site that sets
        // PrecacheOnFlip on travel is still to be found (UD3DRenderDevice::Flush is not
        // called by Browse or LoadMap).
        self.precache_on_flip = true;
    }
}

/// LAN client whose host has left: ReportNetFailure, then UGameEngine::Tick
/// requests "?failed" once the delay has passed; the successful Browse resets ConnectionFailure to 0
/// (after the vtable +0xDC call), so a single travel to the menu and the keys
/// come back (the old behaviour travelled once per frame).
#[cfg(test)]
mod failure_network {
    use crate::Vm;

    #[test]
    fn host_gone_single_return_at_menu() {
        let mut tests = 0;
        for k in 0..100u32 {
            let mut vm = Vm::new();
            // Varied times: appSeconds, ReportNetFailure delay, frame duration.
            vm.app_seconds = 1.0 + k as f64 * 3.7;
            let dt = 1.0 / (20.0 + (k % 7) as f32 * 20.0);
            vm.engine_report_net_failure(5.0 + (k % 3) as f32);
            let mut travels = 0;
            let mut first_one = None;
            for image in 0..2000 {
                vm.app_seconds += dt as f64;
                if let Some(r) = vm.engine_travel_tick_inner(dt) {
                    travels += 1;
                    first_one.get_or_insert((image, r.url.has_option("failed")));
                }
                if vm.ipdrv.connection_failure && vm.ipdrv.time_to_wait_up_to <= vm.app_seconds {
                    vm.client_travel = Some(("?failed".into(), super::TRAVEL_ABSOLUTE, false));
                }
            }
            assert_eq!(travels, 1, "trial {k}: {travels} travels to the menu (first {first_one:?})");
            assert!(!vm.ipdrv.connection_failure, "trial {k}: network failure still flagged (keys ignored)");
            tests += 1;
        }
        assert_eq!(tests, 100);
    }
}
