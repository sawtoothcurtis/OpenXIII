//! Actor channels and replication (Engine.dll UnChan.cpp, UnLevTic.cpp, UnScript.cpp).
//!
//! UActorChannel::SetChannelActor: Recent = the class's default values, its
//! CPF_Config properties reset to zero; RepEval per condition; Retirement per ClassReps.
//! UActorChannel::ReplicateActor: the first time, the class (or the static
//! actor) then compressed Location and Rotation; native list
//! (GetOptimizedRepList) then script properties whose value differs from Recent and whose
//! condition (evaluated once per ConditionIndex) is true; Dirty (lost unreliable bunches).
//! Role goes out under RemoteRole's index and vice versa.
//! UActorChannel::ReceivedBunch: actor creation (SpawnActor, roles swapped),
//! properties (on the host: condition evaluated with roles swapped and bNetOwner required), PostNetReceive,
//! PostNetBeginPlay, then function calls; bTearOff; HandleClientPlayer.
//! AActor::ProcessRemoteFunction and (InternalProcessRemoteFunction).
//! ULevel::ServerTickClient, TickNetServer, TickNetClient.

use crate::net::{ConnState, InBunch, OutBunch, CHTYPE_ACTOR, MAX_CHANNELS};
use crate::net_rep::{net_identical, ClassNetCache, NetField, NetFunc, NetTy};
use crate::netbits::BitWriter;
use crate::types::FuncId;
use crate::value::{ClassId, ObjId, Value};
use crate::Vm;
use std::collections::HashMap;
use std::rc::Rc;
#[path="spawn_record.rs"]mod spawn_record;
use spawn_record::SpawnArchive;

/// Values kept by PreNetReceive (the original's static variables).
pub(crate) struct NetSaved {
    location: Value,
    rotation: Value,
    relative_location: Value,
    relative_rotation: Value,
    base: Value,
    collision_radius: Value,
    collide_actors: Value,
    collision_height: Value,
    sim_anim: Value,
    draw_scale: Value,
    draw_scale_3d: Value,
    draw_type: Value,
    anim_action: Value,
    sim_interpolate: Value,
}

/// FPropertyRetirement.
#[derive(Debug, Clone, Copy)]
pub struct Removes {
    pub in_packet_id: i32,
    pub out_packet_id: i32,
    pub reliable: bool,
}

impl Default for Removes {
    fn default() -> Self {
        Removes { in_packet_id: -1, out_packet_id: -1, reliable: false }
    }
}

/// UActorChannel (beyond UChannel, kept by net.rs).
#[derive(Debug, Default)]
pub struct ActorChannel {
    pub actor: Option<ObjId>,
    pub actor_class: Option<ClassId>,
    pub relevant_time: f64,
    pub last_update_time: f64,
    pub spawn_acked: bool,
    pub actor_dirty: bool,
    pub must_stay_dirty: bool,
    pub still_initial: bool,
    pub recent: Vec<Value>,
    pub rep_eval: Vec<u8>,
    pub dirty: Vec<usize>,
    pub retirement: Vec<Removes>,
    pub broken: bool,
    pub torn_off: bool,
    /// Insertion rank in ActorChannels (TMap: the pairs keep insertion order).
    pub seq: u64,
}

/// Target connection: a client on the host (rank), or the client's connection to the host.
pub type Which = Option<usize>;

impl Vm {
    pub(crate) fn net_conn_ref(&self, which: Which) -> Option<&crate::net_game::NetConn> {
        let d = self.ipdrv.net_driver.as_ref()?;
        match which {
            Some(i) => d.conns.get(i),
            None => d.server_conn.as_ref(),
        }
    }

    /// ULevel::IsServer: neither a driver nor a demo driver with a ServerConnection (true without a driver, as in
    /// the original; UDemoRecDriver is not reimplemented).
    pub(crate) fn net_is_server(&self) -> bool {
        self.ipdrv.net_driver.as_ref().is_none_or(|d| d.server_conn.is_none())
    }

    /// Settings from the driver's ini (UNetDriver::StaticConstructor).
    pub(crate) fn net_drv_cfg(&self) -> crate::ipdrv::NetDriverCfg {
        self.ipdrv.net_driver.as_ref().map(|d| d.cfg.clone()).unwrap_or_default()
    }

    /// The bytes of the packets sent go into the driver's QueuedBytes (UNetConnection::FlushNet).
    pub(crate) fn net_drain_queued(&mut self) {
        let Some(d) = self.ipdrv.net_driver.as_mut() else { return };
        let mut add = 0;
        for c in d.conns.iter_mut().chain(d.server_conn.as_mut()) {
            add += std::mem::take(&mut c.conn.flushed_bytes);
        }
        d.queued_bytes += add;
    }

    /// UNetConnection::IsNetReady.
    pub(crate) fn net_is_ready(&mut self, which: Which, saturate: bool) -> bool {
        self.net_drain_queued();
        let Some(out) = self.net_conn_ref(which).map(|c| c.conn.out.num_bytes() as i32) else { return false };
        let d = self.ipdrv.net_driver.as_mut().unwrap();
        queued_bytes_ready(&mut d.queued_bytes, out, saturate)
    }

    /// UChannel::IsNetReady.
    fn net_channel_ready(&mut self, which: Which, ch: usize) -> bool {
        self.net_channel_ready_sat(which, ch, false)
    }

    /// UChannel::IsNetReady with Saturate: NumOutRec < 127, then UNetConnection::IsNetReady.
    pub(crate) fn net_channel_ready_sat(&mut self, which: Which, ch: usize, saturate: bool) -> bool {
        if self.net_conn_ref(which).is_none_or(|c| c.conn.channel_out_full(ch)) {
            return false;
        }
        self.net_is_ready(which, saturate)
    }

    /// Channel (and acknowledged open) of each actor of the connection.
    fn net_channel_map(&self, which: Which) -> HashMap<ObjId, (usize, bool)> {
        let Some(c) = self.net_conn_ref(which) else { return HashMap::new() };
        c.actor_map.iter().map(|(&a, &ch)| (a, (ch, c.conn.channels[ch].as_ref().is_some_and(|x| x.open_acked)))).collect()
    }

    /// FClassNetCache of the class for the connection's map.
    pub(crate) fn net_cache(&mut self, which: Which, class: ClassId) -> Option<Rc<ClassNetCache>> {
        let index = self.net_conn_ref(which)?.index.clone()?;
        let mut caches = std::mem::take(&mut self.net_conn(which)?.class_caches);
        let r = self.class_net_cache(&index, &mut caches, class);
        if let Some(c) = self.net_conn(which) {
            c.class_caches = caches;
        }
        r
    }

    /// Evaluates replication condition `offset` of the script of `class` for `actor`.
    pub(crate) fn net_eval_condition(&mut self, actor: ObjId, class: ClassId, offset: u16) -> bool {
        let net = self.net_class(class);
        let Some(code) = net.code.clone() else { return false };
        let Some(&i) = code.at.get(&(offset as u32)) else { return false };
        let mut frame = crate::Frame { this: actor, function: None, locals: Vec::new(), package: code.package, switch: None, iterators: Vec::new() };
        match self.eval(&code.statements[i], &mut frame) {
            Ok(v) => v.as_bool(),
            Err(_) => false,
        }
    }

    /// AActor::GetTopOwner: the end of the chain of Owners (the guard of 64 avoids the endless loop
    /// of a cycle, which the original would follow forever).
    pub(crate) fn top_owner(&self, a: ObjId) -> ObjId {
        let mut top = a;
        let mut guard = 0;
        while let Some(o) = self.native_object(top, crate::NativeField::Owner) {
            top = o;
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        top
    }

    /// The player (Player) of the PlayerController at the top of the chain of Owners.
    fn net_top_player(&self, a: ObjId) -> Option<ObjId> {
        let top = self.top_owner(a);
        self.is_a(top, "PlayerController").then(|| self.native_object(top, crate::NativeField::Player)).flatten()
    }

    /// bNetOwner: on the host, the player is this connection; on the client, a viewport.
    fn net_owner_for(&self, which: Which, a: ObjId) -> bool {
        let player = self.net_top_player(a);
        if self.net_is_server() {
            player.is_some() && player == self.net_conn_ref(which).and_then(|c| c.player)
        } else {
            player.and_then(|p| self.objects[p].as_ref()).is_some_and(|o| o.name.starts_with("WindowsViewport"))
        }
    }

    /// UActorChannel::Init and SetChannelActor.
    pub(crate) fn net_set_channel_actor(&mut self, which: Which, ch: usize, actor: ObjId) {
        let Some(class) = self.objects.get(actor).and_then(|o| o.as_ref()).map(|o| o.class) else { return };
        let net = self.net_class(class);
        let conditions = self.net_cache(which, class).map_or(0, |c| c.rep_condition_count.max(0) as usize);
        let time = self.net_time();
        let spawn_priority = self.net_drv_cfg().spawn_priority_seconds;
        let recent = if self.propr(actor, "bNetTemporary").as_bool() {
            Vec::new()
        } else {
            let mut r = self.objects[self.classes[class].cdo].as_ref().map(|o| o.props.to_vec()).unwrap_or_default();
            for &s in &net.config_slots {
                let def = self.classes[class].props[s].clone();
                if let Some(v) = r.get_mut(s) {
                    *v = self.zero(&def.ty, def.dim);
                }
            }
            r
        };
        let ac = ActorChannel {
            actor: Some(actor),
            actor_class: Some(class),
            relevant_time: actor_channel_times(time, spawn_priority).0,
            last_update_time: actor_channel_times(time, spawn_priority).1,
            recent,
            rep_eval: vec![0; conditions],
            retirement: vec![Removes::default(); net.reps.len()],
            ..Default::default()
        };
        if let Some(c) = self.net_conn(which) {
            // TMap::Set: a key already present keeps its place.
            let mut ac = ac;
            ac.seq = match c.actor_map.get(&actor).and_then(|old| c.actor_channels.get(old)) {
                Some(old) => old.seq,
                None => {
                    c.actor_seq += 1;
                    c.actor_seq
                }
            };
            c.actor_channels.insert(ch, ac);
            c.actor_map.insert(actor, ch);
        }
        // Mods build only: the skins a mod gave to this actor (mod_net_skins.rs).
        #[cfg(feature = "mods")]
        self.mod_net_channel_actor(which, ch, actor);
        #[cfg(feature = "mods")]
        if which.is_none() {
            self.mod_net_fx_channel(ch, actor);
        }
    }

    /// Position and rotation of the opening.
    fn net_write_spawn(&mut self, w: &mut BitWriter, a: ObjId, recent:&mut [Value]) {
        let (mut position,mut rotation)=(self.propr(a,"Location").clone(),self.propr(a,"Rotation").clone());
        let slots=(self.prop_slot(a,"Location").ok(),self.prop_slot(a,"Rotation").ok());
        self.spawn_record(SpawnArchive::Write(w),&mut position,&mut rotation,Some((recent,slots.0,slots.1)),true);
    }

    /// UActorChannel::ReplicateActor.
    pub(crate) fn net_replicate_actor(&mut self, which: Which, ch: usize, chmap: &HashMap<ObjId, (usize, bool)>) {
        let Some(mut ac) = self.net_conn(which).and_then(|c| c.actor_channels.remove(&ch)) else { return };
        let Some(actor) = ac.actor.filter(|&a| self.objects.get(a).is_some_and(|o| o.is_some())) else {
            if let Some(c) = self.net_conn(which) {
                c.actor_channels.insert(ch, ac);
            }
            return;
        };
        self.net_replicate_inner(which, ch, actor, &mut ac, chmap);
        if let Some(c) = self.net_conn(which) {
            c.actor_channels.insert(ch, ac);
        }
    }

    fn net_replicate_inner(&mut self, which: Which, ch: usize, actor: ObjId, ac: &mut ActorChannel, chmap: &HashMap<ObjId, (usize, bool)>) {
        let Some(class) = self.objects[actor].as_ref().map(|o| o.class) else { return };
        let (open_packet_id, open_acked, opened_locally) = match self.net_conn_ref(which).and_then(|c| c.conn.channels[ch].as_ref()) {
            Some(x) => (x.open_packet_id, x.open_acked, x.opened_locally),
            None => return,
        };
        let temporary = self.propr(actor, "bNetTemporary").as_bool();
        let (mut b_close, mut b_reliable) = (false, false);
        if open_packet_id == -1 {
            self.set_prop(actor, "bNetInitial", Value::Bool(true));
            b_close = temporary;
            b_reliable = !temporary;
        } else {
            self.set_prop(actor, "bNetInitial", Value::Bool(false));
            if !ac.spawn_acked && open_acked {
                ac.spawn_acked = true;
                for i in (0..ac.retirement.len()).rev() {
                    if ac.retirement[i].out_packet_id != -1 && !ac.retirement[i].reliable && !ac.dirty.contains(&i) {
                        ac.dirty.push(i);
                    }
                }
            }
        }
        let Some(cache) = self.net_cache(which, class) else { return };
        let net = self.net_class(class);
        let owner = self.net_owner_for(which, actor);
        self.set_prop(actor, "bNetOwner", Value::Bool(owner));
        let Some(index) = self.net_conn_ref(which).and_then(|c| c.index.clone()) else { return };
        let mut bunch = match self.net_conn_ref(which) {
            Some(c) => OutBunch::new(&c.conn, ch, b_close),
            None => return,
        };
        bunch.b_reliable = b_reliable;
        let net_initial = self.propr(actor, "bNetInitial").as_bool();
        if net_initial && opened_locally {
            if self.net_dynamic_actor(actor) {
                self.net_write_object(&mut bunch.data, &index, chmap, &Value::Class(Some(class)));
                self.net_write_spawn(&mut bunch.data, actor,&mut ac.recent);
            } else {
                self.net_write_object(&mut bunch.data, &index, chmap, &Value::Object(Some(actor)));
            }
        }
        // Autonomous as seen from anywhere other than its owner: simulated.
        let remote_role = self.propr(actor, "RemoteRole").as_byte();
        if remote_role == 3 {
            let inst = self.po(actor, "Instigator");
            if inst.is_some_and(|i| !self.propr(i, "bNetOwner").as_bool()) && !owner {
                self.set_prop(actor, "RemoteRole", Value::Byte(2));
            }
        }
        let net_dirty = ac.actor_dirty || net_initial;
        self.set_prop(actor, "bNetDirty", Value::Bool(net_dirty));
        self.set_prop(actor, "bNetInitial", Value::Bool(net_initial || ac.still_initial));
        ac.must_stay_dirty = false;
        ac.still_initial = false;
        for e in ac.rep_eval.iter_mut() {
            *e = 0;
        }
        let compare: Vec<Value> = if ac.recent.is_empty() { self.objects[self.classes[class].cdo].as_ref().map(|o| o.props.to_vec()).unwrap_or_default() } else { ac.recent.clone() };
        let (mut reps, stay) = self.optimized_rep_list(actor, &compare, chmap);
        ac.must_stay_dirty |= stay;
        if self.propr(actor, "bNetDirty").as_bool() {
            for fc in cache.rep_properties.iter() {
                let NetField::Prop(p) = &fc.field else { continue };
                let cond = fc.condition.max(0) as usize;
                if ac.rep_eval.get(cond).copied() == Some(2) {
                    continue;
                }
                let cur = self.objects[actor].as_ref().and_then(|o| o.props.get(p.slot).cloned()).unwrap_or(Value::Void);
                let old = compare.get(p.slot).cloned().unwrap_or(Value::Void);
                for idx in 0..p.dim {
                    let (mut src, cmp) = if p.dim > 1 {
                        (match &cur { Value::Array(a) => a.get(idx).cloned().unwrap_or(Value::Void), v => v.clone() }, match &old { Value::Array(a) => a.get(idx).cloned().unwrap_or(Value::Void), v => v.clone() })
                    } else {
                        (cur.clone(), old.clone())
                    };
                    if matches!(p.ty, NetTy::Object(_)) {
                        let serializable = match &src {
                            Value::Object(Some(o)) if self.objects.get(*o).is_some_and(|x| x.is_some()) && self.net_dynamic_actor(*o) => chmap.contains_key(o),
                            _ => true,
                        };
                        if !serializable {
                            ac.must_stay_dirty = true;
                            src = Value::Object(None);
                        }
                    }
                    if !net_identical(&cmp, &src) {
                        let mut eval = ac.rep_eval.get(cond).copied().unwrap_or(0);
                        if eval & 2 == 0 {
                            let (oc, os) = p.rep_owner;
                            let off = self.net_class(oc).props.get(&os).map_or(p.rep_offset, |q| q.rep_offset);
                            let owner_class = self.slot_owner(oc, os);
                            let val = self.net_eval_condition(actor, owner_class, off);
                            eval = val as u8 | 2;
                            if let Some(e) = ac.rep_eval.get_mut(cond) {
                                *e = eval;
                            }
                        }
                        // False condition: the element loop continues.
                        if eval & 1 != 0 {
                            reps.push(p.rep_index + idx);
                        }
                    }
                }
            }
        }
        for i in (0..ac.dirty.len()).rev() {
            let d = ac.dirty[i];
            if !reps.contains(&d) {
                reps.push(d);
            }
        }
        // Role under RemoteRole's index and vice versa (Driver->RoleProperty / RemoteRoleProperty).
        let actor_class = self.class("Actor");
        let role_slot = self.prop_slot(actor, "Role").ok();
        let remote_slot = self.prop_slot(actor, "RemoteRole").ok();
        let mut sent = reps.len();
        let mut filled = false;
        for (k, &ri) in reps.iter().enumerate() {
            let Some(&(slot, elem)) = net.reps.get(ri) else { continue };
            let Some(p) = net.props.get(&slot).cloned() else { continue };
            let field = if Some(slot) == role_slot {
                actor_class.zip(remote_slot).and_then(|(c, s)| cache.from_prop(c, s))
            } else if Some(slot) == remote_slot {
                actor_class.zip(role_slot).and_then(|(c, s)| cache.from_prop(c, s))
            } else {
                cache.from_prop(p.owner, slot)
            };
            let Some(field) = field else { continue };
            let mark = bunch.data.num;
            bunch.data.write_int(field.net_index, cache.max_index());
            if p.dim != 1 {
                bunch.data.write_bytes(&[elem as u8]);
            }
            let cur = self.objects[actor].as_ref().and_then(|o| o.props.get(slot).cloned()).unwrap_or(Value::Void);
            let value = if p.dim > 1 { match &cur { Value::Array(a) => a.get(elem).cloned().unwrap_or(Value::Void), v => v.clone() } } else { cur };
            let mapped = self.net_write_value(&mut bunch.data, &index, chmap, &p.ty, &value);
            if bunch.data.error {
                // FBitWriterMark::Pop.
                bunch.data.error = false;
                for bit in mark..bunch.data.num.min(bunch.data.buffer.len() * 8) {
                    bunch.data.buffer[bit >> 3] &= !(1 << (bit & 7));
                }
                bunch.data.num = mark;
                sent = k;
                filled = true;
                break;
            }
            if !ac.recent.is_empty() {
                let def = self.classes[class].props[slot].clone();
                let stored = if mapped { value } else { self.zero(&def.ty, 1) };
                if p.dim > 1 {
                    if let Some(Value::Array(a)) = ac.recent.get_mut(slot) {
                        if let Some(x) = a.get_mut(elem) {
                            *x = stored;
                        }
                    }
                } else if let Some(x) = ac.recent.get_mut(slot) {
                    *x = stored;
                }
            }
        }
        if bunch.data.num > 0 {
            let reliable = bunch.b_reliable;
            let packet_id = self.net_conn(which).map_or(0, |c| c.conn.send_bunch(bunch, true));
            for &ri in &reps[..sent] {
                ac.dirty.retain(|&d| d != ri);
                if let Some(r) = ac.retirement.get_mut(ri) {
                    r.out_packet_id = packet_id;
                    r.reliable = reliable;
                }
            }
            if temporary {
                if let Some(c) = self.net_conn(which) {
                    c.sent_temporaries.push(actor);
                }
            }
        }
        if !filled {
            ac.last_update_time = self.net_time();
        }
        let initial = self.propr(actor, "bNetInitial").as_bool();
        ac.still_initial = initial && (filled || ac.must_stay_dirty);
        ac.actor_dirty = ac.must_stay_dirty || filled;
        self.set_prop(actor, "bNetOwner", Value::Bool(false));
        self.set_prop(actor, "RemoteRole", Value::Byte(remote_role));
        self.set_prop(actor, "bNetDirty", Value::Bool(false));
    }

    /// UActorChannel::Close: closing bunch (UActorChannel::SetClosingFlag
    /// the actor leaves ActorChannels).
    pub(crate) fn net_close_actor_channel(&mut self, which: Which, ch: usize) {
        let Some(c) = self.net_conn(which) else { return };
        // UChannel::Close only sends the closing bunch (and SetClosingFlag, which takes the actor out of ActorChannels) if the channel is not already closing and the connection is open
        // or pending; UActorChannel::Close sets Actor to 0 in all cases.
        let sent_value = c.conn.channels[ch].as_ref().is_some_and(|x| !x.closing) && matches!(c.conn.state, ConnState::Open | ConnState::Pending);
        c.conn.close_channel(ch);
        if let Some(ac) = c.actor_channels.get_mut(&ch) {
            if let Some(a) = ac.actor.take() {
                if sent_value {
                    c.actor_map.remove(&a);
                }
            }
        }
    }

    /// UActorChannel::Destroy of a channel that has gone (OpenAcked: +0x2C of the channel):
    /// SetClosingFlag, then on the client the actor destroyed (or made autonomous if it is bTearOff,
    /// left if it is bNetTemporary), on the host the actor leaves SentTemporaries if the opening
    /// has not been acknowledged.
    pub(crate) fn net_actor_channel_destroyed(&mut self, which: Which, ch: usize, open_acked: bool) {
        let Some(ac) = self.net_conn(which).and_then(|c| c.actor_channels.remove(&ch)) else { return };
        let Some(a) = ac.actor else { return };
        // UActorChannel::SetClosingFlag: the actor leaves ActorChannels whatever channel the table links it to.
        if let Some(c) = self.net_conn(which) {
            c.actor_map.remove(&a);
        }
        if self.objects.get(a).is_none_or(|o| o.is_none()) {
            return;
        }
        if !self.net_is_server() {
            if self.propr(a, "bTearOff").as_bool() {
                self.set_prop(a, "Role", Value::Byte(4));
                self.set_prop(a, "RemoteRole", Value::Byte(0));
            } else if !self.propr(a, "bNetTemporary").as_bool() {
                let _ = self.destroy_actor_net(a, true);
            }
        } else if !open_acked {
            // RemoveItem removes all occurrences.
            if let Some(c) = self.net_conn(which) {
                c.sent_temporaries.retain(|&x| x != a);
            }
        }
    }

    /// UActorChannel::ReceivedNak, after UChannel::ReceivedNak's resends: the
    /// unreliable properties sent in the lost packet are to be sent again (Dirty,
    /// AddUniqueItem, from last to first), and ActorDirty.
    pub(crate) fn net_actor_channel_nak(&mut self, which: Which, ch: usize, packet_id: i32) {
        let Some(ac) = self.net_conn(which).and_then(|c| c.actor_channels.get_mut(&ch)) else { return };
        if ac.actor_class.is_some() {
            for i in (0..ac.retirement.len()).rev() {
                let r = ac.retirement[i];
                if r.out_packet_id == packet_id && !r.reliable && !ac.dirty.contains(&i) {
                    ac.dirty.push(i);
                }
            }
        }
        ac.actor_dirty = true;
    }

    /// UActorChannel::ReceivedBunch. In tests, the outgoing calls can be stubbed ("rb_*").
    pub(crate) fn net_received_actor_bunch(&mut self, which: Which, bunch: InBunch) {
        let ch = bunch.ch_index;
        let Some(index) = self.net_conn_ref(which).and_then(|c| c.index.clone()) else { return };
        if self.net_conn_ref(which).is_some_and(|c| !c.actor_channels.contains_key(&ch)) {
            let time = self.net_time();
            let prio = self.net_drv_cfg().spawn_priority_seconds;
            if let Some(c) = self.net_conn(which) {
                c.actor_seq += 1;
                let seq = c.actor_seq;
                c.actor_channels.insert(ch, ActorChannel { relevant_time: time, last_update_time: (time as f32 - prio) as f64, seq, ..Default::default() });
            }
        }
        let (broken, torn, has_actor) = match self.net_conn_ref(which).and_then(|c| c.actor_channels.get(&ch)) {
            Some(ac) => (ac.broken, ac.torn_off, ac.actor.is_some()),
            None => return,
        };
        if broken || torn {
            return;
        }
        let mut r = bunch.data.clone();
        let chan_actor = |vm: &Vm, which: Which| -> HashMap<usize, ObjId> {
            vm.net_conn_ref(which).map(|c| c.actor_channels.iter().filter_map(|(&k, a)| a.actor.map(|x| (k, x))).collect()).unwrap_or_default()
        };
        let mut just_spawned = false;
        if !has_actor {
            // GError: fatal error in the original; here the bunch stops.
            if !bunch.b_open {
                let message = format!("New actor channel received non-open packet: {}/{}/{}", bunch.b_open as i32, bunch.b_close as i32, bunch.b_reliable as i32);
                self.log.push(format!("Critical: {message}"));
                self.warn(format!("{message} (fatal error in the original game)"));
                return;
            }
            let actors = chan_actor(self, which);
            let v = self.net_read_object(&mut r, &index, &|c| actors.get(&c).copied(), "");
            let in_actor = match v {
                Value::Object(Some(a)) if self.is_a(a, "Actor") => a,
                Value::Class(Some(c)) if self.class("Actor").is_some_and(|ac| self.is_child_of(c, ac)) => {
                    let(mut loc,mut rot)=(Value::Void,Value::Void);
                    self.spawn_record(SpawnArchive::Read(&mut r),&mut loc,&mut rot,None,true);
                    self.ipdrv.spawn_remote_owned = true;
                    let spawned = self.spawn_actor(c, Value::Object(None), None, loc, rot, Value::Object(None), true);
                    self.ipdrv.spawn_remote_owned = false;
                    match spawned {
                        Ok(Some(a)) => {
                            just_spawned = true;
                            a
                        }
                        // check(InActor) (UnChan.cpp line 813): appFailAssert, fatal
                        // error in the original; here the channel is broken.
                        _ => {
                            let message = "Assertion failed: InActor [File:UnChan.cpp] [Line: 813]";
                            self.log.push(format!("Critical: {message}"));
                            self.warn(format!("{message} (fatal error in the original game)"));
                            if let Some(ac) = self.net_conn(which).and_then(|c| c.actor_channels.get_mut(&ch)) {
                                ac.broken = true;
                            }
                            return;
                        }
                    }
                }
                _ => {
                    self.log.push("Warning: Received invalid actor class".into());
                    if let Some(ac) = self.net_conn(which).and_then(|c| c.actor_channels.get_mut(&ch)) {
                        ac.broken = true;
                    }
                    return;
                }
            };
            self.net_set_channel_actor(which, ch, in_actor);
        }
        let Some(actor) = self.net_conn_ref(which).and_then(|c| c.actor_channels.get(&ch)).and_then(|a| a.actor) else { return };
        let Some(class) = self.objects.get(actor).and_then(|o| o.as_ref()).map(|o| o.class) else { return };
        let Some(cache) = self.net_cache(which, class) else { return };
        let net = self.net_class(class);
        let owner = self.net_owner_for(which, actor);
        self.set_prop(actor, "bNetOwner", Value::Bool(owner));
        let server = self.net_is_server();
        let read_field = |r: &mut crate::netbits::BitReader| {
            let i = r.read_int(cache.max_index());
            if r.error {
                None
            } else {
                cache.from_index(i).cloned()
            }
        };
        let mut fc = read_field(&mut r);
        while let Some(f) = fc.clone() {
            let saved = Some(self.net_pre_receive(actor));
            // Properties.
            while let Some(NetField::Prop(p)) = fc.as_ref().map(|x| x.field.clone()) {
                let elem = if p.dim != 1 { r.read_bytes(1)[0] as usize } else { 0 };
                let mut keep = true;
                if server {
                    self.net_swap_roles(actor);
                    let val = self.net_eval_condition(actor, p.owner, p.rep_offset);
                    self.net_swap_roles(actor);
                    if !val || !self.propr(actor, "bNetOwner").as_bool() {
                        self.log.push(format!("DevNet: Received unwanted property value {} in {}", p.name, self.path_name(actor)));
                        keep = false;
                    }
                }
                let ri = p.rep_index + elem;
                if let Some(ac) = self.net_conn(which).and_then(|c| c.actor_channels.get_mut(&ch)) {
                    if let Some(ret) = ac.retirement.get_mut(ri) {
                        if bunch.packet_id >= ret.in_packet_id {
                            ret.in_packet_id = bunch.packet_id;
                        } else {
                            keep = false;
                        }
                    }
                }
                if !keep && self.net_conn_ref(which).and_then(|c| c.actor_channels.get(&ch)).and_then(|a| a.retirement.get(ri)).is_some_and(|ret| bunch.packet_id < ret.in_packet_id) {
                    self.log.push(format!("DevNet: Received out-of-date {}", p.name));
                }
                let cur = self.objects[actor].as_ref().and_then(|o| o.props.get(p.slot).cloned()).unwrap_or(Value::Void);
                let old = if p.dim > 1 { match &cur { Value::Array(a) => a.get(elem).cloned().unwrap_or(Value::Void), v => v.clone() } } else { cur.clone() };
                let actors = chan_actor(self, which);
                let new = self.net_read_value(&mut r, &index, &|c| actors.get(&c).copied(), &p.ty, &old);
                if keep {
                    if let Some(o) = self.objects[actor].as_mut() {
                        if p.dim > 1 {
                            if let Some(Value::Array(a)) = o.props.get_mut(p.slot) {
                                if let Some(x) = a.get_mut(elem) {
                                    *x = new.clone();
                                }
                            }
                        } else if let Some(x) = o.props.get_mut(p.slot) {
                            *x = new.clone();
                        }
                    }
                    if let Some(ac) = self.net_conn(which).and_then(|c| c.actor_channels.get_mut(&ch)) {
                        if !ac.recent.is_empty() {
                            if p.dim > 1 {
                                if let Some(Value::Array(a)) = ac.recent.get_mut(p.slot) {
                                    if let Some(x) = a.get_mut(elem) {
                                        *x = new;
                                    }
                                }
                            } else if let Some(x) = ac.recent.get_mut(p.slot) {
                                *x = new;
                            }
                        }
                    }
                }
                fc = read_field(&mut r);
            }
            let _ = (&f, &net);
            {
                if let Some(saved) = saved {
                    self.net_post_receive(actor, saved);
                }
            }
            if just_spawned {
                just_spawned = false;
                {
                    self.event_pub(actor, "PostNetBeginPlay", vec![]);
                }
            }
            // Function calls.
            match fc.as_ref().map(|x| x.field.clone()) {
                Some(NetField::Func(func)) => {
                    let actors = chan_actor(self, which);
                    self.net_receive_rpc(actor, &func, &mut r, &index, &actors, server);
                    fc = read_field(&mut r);
                }
                Some(NetField::Prop(_)) => {}
                None => break,
            }
            if self.objects.get(actor).is_none_or(|o| o.is_none()) {
                return;
            }
        }
        // bTearOff on the client.
        let level_mode = self.level.map_or(0, |l| self.propr(l, "NetMode").as_byte());
        if self.objects.get(actor).is_some_and(|o| o.is_some()) && self.propr(actor, "bTearOff").as_bool() && level_mode == 3 {
            self.set_prop(actor, "Role", Value::Byte(4));
            self.set_prop(actor, "RemoteRole", Value::Byte(0));
            if let Some(c) = self.net_conn(which) {
                c.actor_map.remove(&actor);
                if let Some(ac) = c.actor_channels.get_mut(&ch) {
                    ac.torn_off = true;
                    ac.actor = None;
                }
            }
            // No eventTornOff here in XIII (roles, bTornOff, ActorChannels.Remove, Actor = NULL).
            return;
        }
        // UNetConnection::HandleClientPlayer.
        if which.is_none()
            && self.net_conn_ref(None).is_some_and(|c| c.conn.state == ConnState::Pending)
            && self.objects.get(actor).is_some_and(|o| o.is_some())
            && self.propr(actor, "bNetOwner").as_bool()
            && self.is_a(actor, "PlayerController")
        {
            self.net_handle_client_player(actor);
        }
    }

    fn net_swap_roles(&mut self, a: ObjId) {
        let (r, rr) = (self.propr(a, "Role").clone(), self.propr(a, "RemoteRole").clone());
        self.set_prop(a, "Role", rr);
        self.set_prop(a, "RemoteRole", r);
    }

    /// Function call received (end of UActorChannel::ReceivedBunch).
    fn net_receive_rpc(&mut self, actor: ObjId, func: &Rc<NetFunc>, r: &mut crate::netbits::BitReader, index: &crate::net_map::MapIndex, actors: &HashMap<usize, ObjId>, server: bool) {
        let Some(f) = self.find_function(actor, &func.name, false) else { return };
        // Condition of the function (its first declaration) before reading the parameters.
        let mut skip = false;
        if server {
            self.net_swap_roles(actor);
            let val = self.net_eval_condition(actor, func.owner, func.rep_offset);
            self.net_swap_roles(actor);
            if !val || !self.propr(actor, "bNetOwner").as_bool() {
                self.log.push(format!("DevNet: Received unwanted function {} in {}", func.name, self.path_name(actor)));
                skip = true;
            }
        }
        let mut args: Vec<Option<Value>> = Vec::new();
        for (local, dim, ty, is_bool) in &func.params {
            let def = self.functions[f].locals.get(*local).cloned();
            let zero = def.as_ref().map_or(Value::Void, |d| self.zero(&d.ty, d.dim));
            let read_from = *is_bool || r.read_bit();
            let v = if read_from { self.net_read_value(r, index, &|c| actors.get(&c).copied(), ty, &zero) } else { zero };
            let _ = dim;
            args.push(Some(v));
        }
        if !skip {
            self.set_prop(actor, "bClientDemoNetFunc", Value::Bool(true));
            // Parameters in the order of the locals (the ParamDefs follow the Children).
            let params = self.functions[f].params.clone();
            let mut call_args: Vec<Option<Value>> = vec![None; params.len()];
            for (k, (local, ..)) in func.params.iter().enumerate() {
                if let Some(pos) = params.iter().position(|p| p.local == *local) {
                    call_args[pos] = args[k].take();
                }
            }
            if let Err(e) = self.call(f, actor, call_args) {
                self.warn(format!("RPC {}: {e}", func.name));
            }
        }
    }

    /// AActor::PreNetReceive / APawn::PreNetReceive.
    /// PreNetReceive before the properties of a bunch: AActor keeps Location,
    /// Rotation, RelativeLocation, RelativeRotation, Base, CollisionRadius, bCollideActors,
    /// CollisionHeight, SimAnim, DrawScale, DrawScale3D, DrawType and takes the actor out of the collision
    /// hash if it blocks; APawn keeps AnimAction, AMover
    /// SimInterpolate, ATheDuck OldLoopAnimID = LoopAnimID.
    fn net_pre_receive(&mut self, a: ObjId) -> NetSaved {
        let g = |vm: &Vm, n: &str| vm.propr(a, n).clone();
        if self.is_a(a, "TheDuck") {
            let l = g(self, "LoopAnimID");
            self.set_prop(a, "OldLoopAnimID", l);
        }
        let s = NetSaved {
            location: g(self, "Location"),
            rotation: g(self, "Rotation"),
            relative_location: g(self, "RelativeLocation"),
            relative_rotation: g(self, "RelativeRotation"),
            base: g(self, "Base"),
            collision_radius: g(self, "CollisionRadius"),
            collide_actors: g(self, "bCollideActors"),
            collision_height: g(self, "CollisionHeight"),
            sim_anim: g(self, "SimAnim"),
            draw_scale: g(self, "DrawScale"),
            draw_scale_3d: g(self, "DrawScale3D"),
            draw_type: g(self, "DrawType"),
            anim_action: if self.is_a(a, "Pawn") { g(self, "AnimAction") } else { Value::Void },
            sim_interpolate: if self.is_a(a, "Mover") { g(self, "SimInterpolate") } else { Value::Void },
        };
        if self.propr(a, "bCollideActors").as_bool() {
            self.octree_remove(a);
        }
        s
    }

    /// PostNetReceive of the actor's class, after its properties.
    fn net_post_receive(&mut self, a: ObjId, mut s: NetSaved) {
        self.actor_post_net_receive(a, &mut s);
        if self.is_a(a, "Pawn") {
            self.pawn_post_net_receive(a, &s);
            if self.is_a(a, "TheDuck") {
                // ATheDuck::PostNetReceive: received LoopAnimID set back to the old one,
                // SetLoopAnimation(received), LaunchSound.
                let (new, old) = (self.propr(a, "LoopAnimID").clone(), self.propr(a, "OldLoopAnimID").clone());
                self.set_prop(a, "LoopAnimID", old);
                self.set_prop(a, "OldLoopAnimID", new.clone());
                self.event_pub(a, "SetLoopAnimation", vec![new]);
                self.event_pub(a, "LaunchSound", vec![]);
            }
        } else if self.is_a(a, "Mover") {
            self.mover_post_net_receive(a, &s);
        } else if self.is_a(a, "PlayerReplicationInfo") {
            self.pri_post_net_receive(a);
        } else if self.is_a(a, "WeaponAttachment") {
            self.weapon_attachment_post_net_receive(a);
        } else if self.is_a(a, "Pickup") {
            self.pickup_post_net_receive(a);
        }
    }

    /// APickup::PostNetReceive, after AActor::PostNetReceive: bOnlyReplicateHidden, ClientPickedUp if
    /// hidden, otherwise ClientRespawned, then RefreshDynDisp (XLevel, vtable +0x10C).
    pub(crate) fn pickup_post_net_receive(&mut self, a: ObjId) {
        if self.propr(a, "bOnlyReplicateHidden").as_bool() {
            let ev = if self.propr(a, "bHidden").as_bool() { "ClientPickedUp" } else { "ClientRespawned" };
            self.event_pub(a, ev, vec![]);
            self.refresh_dyn_disp(a);
        }
    }

    /// AActor::PostNetReceive: the kept values come back and the received ones go
    /// through the engine: PlayReplicatedAnim, PostNetReceiveLocation, MoveActor (rotation),
    /// SetCollisionSize, SetCollision, SetDrawScale, SetDrawScale3D, SetDrawType, SetBase (Bump
    /// on both sides), then the relative position and rotation on a moving support;
    /// bJustTeleported to false. `s` receives the received values.
    fn actor_post_net_receive(&mut self, a: ObjId, s: &mut NetSaved) {
        let swap = |vm: &mut Vm, n: &str, v: &mut Value| {
            let cur = vm.propr(a, n).clone();
            vm.set_prop(a, n, std::mem::replace(v, cur));
        };
        swap(self, "Location", &mut s.location);
        swap(self, "Rotation", &mut s.rotation);
        swap(self, "RelativeLocation", &mut s.relative_location);
        swap(self, "RelativeRotation", &mut s.relative_rotation);
        swap(self, "Base", &mut s.base);
        swap(self, "bCollideActors", &mut s.collide_actors);
        swap(self, "CollisionRadius", &mut s.collision_radius);
        swap(self, "CollisionHeight", &mut s.collision_height);
        swap(self, "DrawScale", &mut s.draw_scale);
        swap(self, "DrawScale3D", &mut s.draw_scale_3d);
        swap(self, "DrawType", &mut s.draw_type);
        if self.propr(a, "bCollideActors").as_bool() {
            self.octree_add(a);
        }
        if !matches!(self.propr(a, "Mesh"), Value::Object(None) | Value::Void) && sim_anim_changed(&self.propr(a, "SimAnim").clone(), &s.sim_anim) {
            self.play_replicated_anim(a);
        }
        if self.propr(a, "Location") != &s.location {
            let l = s.location.as_vector();
            if self.is_a(a, "Pawn") {
                self.pawn_post_net_receive_location(a, l);
            } else {
                self.actor_post_net_receive_location(a, l);
            }
        }
        if self.propr(a, "Rotation") != &s.rotation {
            let mut hit = crate::physics::CheckResult::new(1.0);
            let r = s.rotation.as_rotator();
            self.move_actor(a, [0.0; 3], r, &mut hit, false, false, false, true);
        }
        let (cr, ch) = (s.collision_radius.as_float(), s.collision_height.as_float());
        if self.propr(a, "CollisionRadius").as_float() != cr || self.propr(a, "CollisionHeight").as_float() != ch {
            self.set_collision_size(a, cr, ch);
        }
        if self.propr(a, "bCollideActors").as_bool() != s.collide_actors.as_bool() {
            let (b, p) = (self.propr(a, "bBlockActors").as_bool(), self.propr(a, "bBlockPlayers").as_bool());
            self.set_collision(a, s.collide_actors.as_bool(), b, p);
        }
        for (k, native, v) in [("DrawScale", "SetDrawScale", &s.draw_scale), ("DrawScale3D", "SetDrawScale3D", &s.draw_scale_3d), ("DrawType", "SetDrawType", &s.draw_type)] {
            if self.propr(a, k) != v {
                let mut args = [v.clone()];
                let _ = crate::natives::call(self, native, a, &mut args, &[true]);
            }
        }
        if self.propr(a, "Base") != &s.base {
            let new_base = s.base.as_object();
            if let Some(b) = new_base {
                // eventBump directly, on both sides: not APawn::NotifyBump (the controller first).
                self.event_pub(a, "Bump", vec![Value::Object(Some(b))]);
                self.event_pub(b, "Bump", vec![Value::Object(Some(a))]);
            }
            let loc_changed = self.propr(a, "RelativeLocation") != &s.relative_location;
            let rot_changed = self.propr(a, "RelativeRotation") != &s.relative_rotation;
            self.set_base_full(a, new_base, [0.0, 0.0, 1.0], true);
            if !loc_changed {
                s.relative_location = self.propr(a, "RelativeLocation").clone();
            }
            if !rot_changed {
                s.relative_rotation = self.propr(a, "RelativeRotation").clone();
            }
        }
        let base = self.native_object(a, crate::NativeField::Base).filter(|&b| !self.propr(b, "bWorldGeometry").as_bool());
        if let Some(b) = base {
            let bone = self.propr(a, "AttachmentBone").as_name();
            if bone.is_empty() || bone.eq_ignore_ascii_case("None") {
                if self.propr(a, "RelativeLocation") != &s.relative_location {
                    let (bl, rl) = (self.propr(b, "Location").as_vector(), s.relative_location.as_vector());
                    self.far_move_actor(a, [rl[0] + bl[0], rl[1] + bl[1], rl[2] + bl[2]], false, true, true);
                }
                if self.propr(a, "RelativeRotation") != &s.relative_rotation {
                    // (GMath.UnitCoords / RelativeRotation) * (GMath.UnitCoords / Base.Rotation)
                    // transposed, then OrthoRotation.
                    use openxiii_assets::coords::{div_rotator, UNIT};
                    let c1 = div_rotator(&UNIT, self.propr(b, "Rotation").as_rotator());
                    let c2 = div_rotator(&UNIT, s.relative_rotation.as_rotator()).mul(&c1.transpose());
                    let r = crate::world::ortho_rotation([c2.x, c2.y, c2.z]);
                    let mut hit = crate::physics::CheckResult::new(1.0);
                    self.move_actor(a, [0.0; 3], r, &mut hit, false, false, false, true);
                }
            } else {
                self.set_prop(a, "RelativeLocation", s.relative_location.clone());
                self.set_prop(a, "RelativeRotation", s.relative_rotation.clone());
            }
        }
        self.set_prop(a, "bJustTeleported", Value::Bool(false));
    }

    /// AActor::PostNetReceiveLocation: XLevel->FarMoveActor(this, received Location, 0, 1, 1).
    pub(crate) fn actor_post_net_receive_location(&mut self, a: ObjId, l: [f32; 3]) {
        self.far_move_actor(a, l, false, true, true);
    }

    /// APawn::PostNetReceiveLocation: a simulated pawn that moves and receives a position
    /// less than 100 units away does not jump there: within two radii, it gets 15 % closer in
    /// X and Y; further away, moveSmooth by 35 %, then 50 % more in X and Y if more than
    /// three quarters remain; the spot (X, Y, received Z) is taken if SinglePointCheck finds it free,
    /// otherwise the received position (24-bit FPU, sums (Z² + Y²) + X²).
    pub(crate) fn pawn_post_net_receive_location(&mut self, a: ObjId, new: [f32; 3]) {
        let default = |vm: &mut Vm| {
            vm.far_move_actor(a, new, false, true, true);
        };
        // Velocity nearly zero (|c| < 1e-4 for each component).
        let v = self.propr(a, "Velocity").as_vector();
        let still = v.iter().all(|c| (c.abs() as f64) < 1e-4);
        if self.propr(a, "Role").as_byte() != 2 || still {
            return default(self);
        }
        let d2 = |l: [f32; 3]| {
            let (dx, dy, dz) = (l[0] - new[0], l[1] - new[1], l[2] - new[2]);
            (dz * dz + dy * dy) + dx * dx
        };
        let loc = self.propr(a, "Location").as_vector();
        let dist = d2(loc);
        if !(dist < 10000.0) {
            return default(self);
        }
        let (mut ax, mut ay) = (loc[0], loc[1]);
        let cr = self.propr(a, "CollisionRadius").as_float();
        let (ddx, ddy) = (new[0] - loc[0], new[1] - loc[1]);
        if !(dist > (cr * cr) * 4.0) {
            ax = ddx * 0.15 + ax;
            ay = ddy * 0.15 + ay;
        } else {
            let ddz = new[2] - loc[2];
            self.move_smooth(a, [ddx * 0.35, ddy * 0.35, ddz * 0.35]);
            let l2 = self.propr(a, "Location").as_vector();
            if dist * 0.75 < d2(l2) {
                ax = (new[0] - l2[0]) * 0.5 + ax;
                ay = ay + (new[1] - l2[1]) * 0.5;
            }
        }
        let spot = [ax, ay, new[2]];
        let ch = self.propr(a, "CollisionHeight").as_float();
        let mut hit = crate::physics::CheckResult::new(1.0);
        if self.single_point_check(&mut hit, spot, [cr, cr, ch], 0, true, false) {
            self.far_move_actor(a, spot, false, true, true);
        } else {
            default(self);
        }
    }

    /// APawn::PostNetReceive: received AnimAction set back to the old one then
    /// SetAnimAction(received); PlayBaffed if BaffeCount changes; ChangeSkin if SkinID changes.
    fn pawn_post_net_receive(&mut self, a: ObjId, s: &NetSaved) {
        let aa = self.propr(a, "AnimAction").clone();
        if !net_identical(&aa, &s.anim_action) {
            self.set_prop(a, "AnimAction", s.anim_action.clone());
            self.event_pub(a, "SetAnimAction", vec![aa]);
        }
        if self.propr(a, "BaffeCount").as_int() != self.propr(a, "MemBaffeCount").as_int() {
            let b = self.propr(a, "BaffeCount").clone();
            self.set_prop(a, "MemBaffeCount", b);
            self.event_pub(a, "PlayBaffed", vec![]);
        }
        if self.propr(a, "OldSkinID").as_int() != self.propr(a, "SkinID").as_int() {
            let s = self.propr(a, "SkinID").clone();
            self.set_prop(a, "OldSkinID", s);
            self.event_pub(a, "ChangeSkin", vec![]);
        }
    }

    /// AMover::PostNetReceive: SimInterpolate changed: PhysAlpha and PhysRate
    /// (× 0.01), OldPos, OldRot, KeyNum and PrevKeyNum (_ftol of SimInterpolate.Z, low byte and
    /// the next one), setPhysics(PHYS_MovingBrush), bInterpolating.
    fn mover_post_net_receive(&mut self, a: ObjId, s: &NetSaved) {
        let si = self.propr(a, "SimInterpolate").as_vector();
        if Value::vector(si[0], si[1], si[2]) == s.sim_interpolate {
            return;
        }
        self.set_prop(a, "PhysAlpha", Value::Float(si[0] * 0.01));
        let p = self.propr(a, "SimOldPos").clone();
        self.set_prop(a, "OldPos", p);
        self.set_prop(a, "PhysRate", Value::Float(si[1] * 0.01));
        let r = [self.propr(a, "SimOldRotPitch").as_int(), self.propr(a, "SimOldRotYaw").as_int(), self.propr(a, "SimOldRotRoll").as_int()];
        self.set_prop(a, "OldRot", Value::rotator(r[0], r[1], r[2]));
        let k = crate::value::ftol(si[2]) as i32; // _ftol: 64-bit fistp, EAX kept
        self.set_prop(a, "KeyNum", Value::Byte(k as u8));
        self.set_prop(a, "PrevKeyNum", Value::Byte((k >> 8) as u8));
        self.set_physics(a, 8, None, [0.0, 0.0, 1.0]);
        self.set_prop(a, "bInterpolating", Value::Bool(true));
    }

    /// APlayerReplicationInfo::PostNetReceive, on the client: Ping of the local player
    /// minus half the average frame duration (ms), at least 0; OldName / PreviousName;
    /// ScoreUpdated; SkinUpdated (log "PostNetReceive SkinCodeName").
    fn pri_post_net_receive(&mut self, a: ObjId) {
        if self.level.is_none_or(|l| self.propr(l, "NetMode").as_byte() != 3) {
            return;
        }
        let local_pri = self.find_viewport_object(0).and_then(|v| self.po(v, "Actor")).and_then(|pc| self.po(pc, "PlayerReplicationInfo"));
        if self.ipdrv.net_driver.is_some() && local_pri == Some(a) {
            let avg = self.net_conn_ref(None).map_or(0.0, |c| c.conn.average_frame_time) as f32;
            let adj = ((avg * 1000.0) * -0.5) as i32;
            let ping = self.propr(a, "Ping").as_int().wrapping_add(adj);
            self.set_prop(a, "Ping", Value::Int(ping));
        }
        if self.propr(a, "Ping").as_int() < 0 {
            self.set_prop(a, "Ping", Value::Int(0));
        }
        let prev = self.propr(a, "PreviousName").clone();
        let name = self.propr(a, "PlayerName").clone();
        if !net_identical(&prev, &name) {
            self.set_prop(a, "OldName", prev);
            self.set_prop(a, "PreviousName", name);
        }
        if self.propr(a, "Score").as_float() != self.propr(a, "OldScore").as_float() {
            let sc = self.propr(a, "Score").clone();
            self.set_prop(a, "OldScore", sc);
            // APlayerReplicationInfo::eventScoreUpdated.
            self.event_pub(a, "ScoreUpdated", vec![]);
        }
        let skin = self.propr(a, "SkinCodeName").clone();
        if !net_identical(&self.propr(a, "OldSkinCodeName").clone(), &skin) {
            // FString passed by value to Logf ("%s"): an empty string has a null Data, which msvcr70's vsnprintf writes as "(null)".
            let text_value = skin.as_name();
            self.log.push(format!("Log: PostNetReceive SkinCodeName {}", if text_value.is_empty() { "(null)" } else { text_value.as_str() }));
            self.set_prop(a, "OldSkinCodeName", skin);
            // APlayerReplicationInfo::eventSkinUpdated.
            self.event_pub(a, "SkinUpdated", vec![]);
        }
    }

    /// AWeaponAttachment::PostNetReceive: on another player's pawn
    /// (Instigator not controlled here: APawn::IsLocallyControlled), FlashCount,
    /// AltFlashCount and ReloadClientCount copied, with ThirdPersonEffects, ThirdPersonAltEffects
    /// and ThirdPersonReLoad.
    pub(crate) fn weapon_attachment_post_net_receive(&mut self, a: ObjId) {
        let Some(inst) = self.po(a, "Instigator").filter(|&p| self.alive(p)) else { return };
        let local = self.native_object(inst, crate::NativeField::Controller).is_some_and(|c| self.is_a(c, "PlayerController") && self.native_object(c, crate::NativeField::Player).is_some());
        if local {
            return;
        }
        for (k, ev) in [("FlashCount", "ThirdPersonEffects"), ("AltFlashCount", "ThirdPersonAltEffects"), ("ReloadClientCount", "ThirdPersonReLoad")] {
            let v = self.propr(a, k).clone();
            if self.propr(inst, k).as_byte() != v.as_byte() {
                self.set_prop(inst, k, v);
                self.event_pub(a, ev, vec![]);
            }
        }
    }

    /// UNetConnection::HandleClientPlayer: the received controller becomes the one of the
    /// first viewport; the connection opens.
    pub(crate) fn net_handle_client_player(&mut self, pc: ObjId) {
        let Some(viewport) = self.active_viewport_objects().first().copied() else { return };
        if let Some(old) = self.po(viewport, "Actor") {
            self.set_prop(old, "Player", Value::Object(None));
        }
        let speed = self.net_conn_ref(None).map_or(0, |c| c.conn.current_net_speed);
        self.set_prop(viewport, "CurrentNetSpeed", Value::Int(speed));
        self.set_prop(pc, "Role", Value::Byte(3));
        self.set_prop(pc, "ShowFlags", Value::Int(0x30cc80c));
        self.set_prop(pc, "RendMap", Value::Byte(5));
        let stub = false;
        if !stub {
            self.attach_player(pc, Some(viewport));
        }
        let (a, b) = (self.objects[pc].as_ref().map(|o| o.name.clone()).unwrap_or_default(), self.objects[viewport].as_ref().map(|o| o.name.clone()).unwrap_or_default());
        self.log.push(format!("Log: {a} setplayer {b}"));
        if let Some(slot) = self.engine.client.viewports.first().copied() {
            self.reset_slot_input(slot);
        }
        if let Some(l) = self.level {
            self.set_prop(l, "LevelAction", Value::Byte(0));
        }
        if let Some(c) = self.net_conn(None) {
            c.pc = Some(pc);
            c.conn.state = ConnState::Open;
        }
    }

    /// AActor::ProcessRemoteFunction before a function is run: Some(true)
    /// if it does not run here (sent, or absorbed by a proxy).
    pub(crate) fn net_process_remote_function(&mut self, f: FuncId, this: ObjId, locals: &[Value]) -> bool {
        let flags = self.functions[f].flags;
        use openxiii_script::objects::func;
        if flags & func::STATIC != 0 || !self.is_a(this, "Actor") || self.propr(this, "bDeleteMe").as_bool() {
            return false;
        }
        let role = self.propr(this, "Role").as_byte();
        let absorb = role < 3 && flags & (func::SIMULATED | func::NATIVE) == 0;
        let mode = self.level.map_or(0, |l| self.propr(l, "NetMode").as_byte());
        if mode == 0 {
            return false;
        }
        if flags & func::NET == 0 {
            return absorb;
        }
        let mut which: Which = None;
        if role == 4 {
            // GetTopOwner, Cast<APlayerController>, Cast<UNetConnection>(Player)
            let player = self.net_top_player(this);
            let conn = player.and_then(|p| self.ipdrv.net_driver.as_ref().and_then(|d| d.conns.iter().position(|c| c.player == Some(p))));
            match conn {
                Some(i) => which = Some(i),
                None => return absorb,
            }
        }
        // The function that carries the condition: the highest in the chain.
        let Some((owner, net_func)) = self.net_top_function(f) else { return absorb };
        if !self.net_eval_condition(this, owner, net_func.rep_offset) {
            return absorb;
        }
        let is_server = mode == 1 || mode == 2;
        if !is_server {
            which = None;
            if self.ipdrv.net_driver.as_ref().is_none_or(|d| d.server_conn.is_none()) {
                return true;
            }
        }
        if flags & func::NET_RELIABLE == 0 && !self.net_is_ready(which, false) {
            return true;
        }
        self.net_send_rpc(this, which, &net_func, locals, is_server);
        true
    }

    /// Deliberate difference in OpenXIII (dedicated server): ClientTravel is static
    /// (0x800), so the original never sends it and crashes at the CastChecked<UViewport> of a remote player
    /// (ProcessServerTravel with clients). Here the function (Net, NetReliable, RepOffset) goes to the player's connection
    /// like an ordinary RPC, so that clients follow the map change. True if sent.
    pub(crate) fn net_send_static_to_client(&mut self, f: FuncId, this: ObjId, locals: &[Value]) -> bool {
        let player = self.net_top_player(this);
        let Some(which) = player.and_then(|p| self.ipdrv.net_driver.as_ref().and_then(|d| d.conns.iter().position(|c| c.player == Some(p)))) else { return false };
        let Some((owner, net_func)) = self.net_top_function(f) else { return false };
        if !self.net_eval_condition(this, owner, net_func.rep_offset) {
            return false;
        }
        self.net_send_rpc(this, Some(which), &net_func, locals, true);
        true
    }

    /// Replicated function that declares `f` (UFunction::GetSuperFunction up to the top).
    fn net_top_function(&mut self, f: FuncId) -> Option<(ClassId, Rc<NetFunc>)> {
        let name = self.functions[f].name.to_lowercase();
        let mut c = Some(self.functions[f].owner);
        let mut top = None;
        while let Some(ci) = c {
            if self.classes[ci].functions.contains_key(&name) {
                top = Some(ci);
            }
            c = self.classes[ci].super_class;
        }
        let owner = top?;
        let net = self.net_class(owner);
        net.own.iter().find_map(|x| match x {
            NetField::Func(nf) if nf.name.eq_ignore_ascii_case(&name) => Some((owner, nf.clone())),
            _ => None,
        })
    }

    /// (InternalProcessRemoteFunction).
    fn net_send_rpc(&mut self, actor: ObjId, which: Which, func: &Rc<NetFunc>, locals: &[Value], is_server: bool) {
        let Some(class) = self.objects.get(actor).and_then(|o| o.as_ref()).map(|o| o.class) else { return };
        let Some(cache) = self.net_cache(which, class) else { return };
        let field = NetField::Func(func.clone());
        let Some(fc) = cache.from_field(&field).cloned() else { return };
        let mut ch = self.net_conn_ref(which).and_then(|c| c.actor_map.get(&actor).copied());
        if ch.is_none() {
            if !is_server {
                return;
            }
            let Some(new) = self.net_conn(which).and_then(|c| c.conn.create_channel(CHTYPE_ACTOR, true, None)) else { return };
            self.net_set_channel_actor(which, new, actor);
            ch = Some(new);
        }
        let ch = ch.unwrap();
        if self.net_conn_ref(which).and_then(|c| c.conn.channels[ch].as_ref()).is_some_and(|x| x.open_packet_id == -1) {
            if !is_server {
                return;
            }
            let stub_value = false;
            if !stub_value {
                let chmap = self.net_channel_map(which);
                self.net_replicate_actor(which, ch, &chmap);
            }
        }
        let Some(index) = self.net_conn_ref(which).and_then(|c| c.index.clone()) else { return };
        let mut bunch = match self.net_conn_ref(which) {
            Some(c) => OutBunch::new(&c.conn, ch, false),
            None => return,
        };
        bunch.data.write_int(fc.net_index, cache.max_index());
        let chmap = self.net_channel_map(which);
        let f = func.func;
        for (local, _dim, ty, is_bool) in &func.params {
            let v = locals.get(*local).cloned().unwrap_or(Value::Void);
            let send = if *is_bool {
                true
            } else {
                let def = self.functions[f].locals.get(*local).cloned();
                let zero = def.as_ref().map_or(Value::Void, |d| self.zero(&d.ty, d.dim));
                let differs = !net_identical(&v, &zero);
                bunch.data.write_bit(differs);
                differs
            };
            if send {
                self.net_write_value(&mut bunch.data, &index, &chmap, ty, &v);
            }
        }
        if func.flags & openxiii_script::objects::func::NET_RELIABLE != 0 {
            bunch.b_reliable = true;
        }
        if bunch.data.error {
            self.log.push("DevNet: RPC bunch overflowed".into());
            return;
        }
        if let Some(c) = self.net_conn(which) {
            c.conn.send_bunch(bunch, true);
        }
    }

    /// UNetDriver::NotifyActorDestroyed: the actor's channels close.
    pub(crate) fn net_notify_actor_destroyed(&mut self, a: ObjId) {
        let Some(n) = self.ipdrv.net_driver.as_ref().map(|d| d.conns.len()) else { return };
        for i in (0..n).rev() {
            if self.propr(a, "bNetTemporary").as_bool() {
                if let Some(c) = self.net_conn(Some(i)) {
                    c.sent_temporaries.retain(|&x| x != a);
                }
            }
            if let Some(ch) = self.net_conn_ref(Some(i)).and_then(|c| c.actor_map.get(&a).copied()) {
                self.net_close_actor_channel(Some(i), ch);
            }
        }
    }

    /// ULevel::TickNetClient: the local player's pawn goes back to the host; a
    /// closed connection ends the game ("ConnectionFailed").
    pub(crate) fn net_tick_client(&mut self) {
        let Some(state) = self.ipdrv.net_driver.as_ref().and_then(|d| d.server_conn.as_ref()).map(|c| c.conn.state) else { return };
        if self.ipdrv.pending.is_some() {
            return;
        }
        match state {
            ConnState::Open => {
                // Mods build only: the scripts' effects kept at a character follow it (mod_net_fx.rs).
                #[cfg(feature = "mods")]
                self.mod_net_fx_follow();
                // Order of the pairs of ActorChannels (insertion).
                let mut chans: Vec<(u64, usize, ObjId)> = self.net_conn_ref(None).map(|c| c.actor_channels.iter().filter_map(|(&k, a)| a.actor.map(|x| (a.seq, k, x))).collect()).unwrap_or_default();
                chans.sort();
                let chans: Vec<(usize, ObjId)> = chans.into_iter().map(|(_, k, x)| (k, x)).collect();
                let chmap = self.net_channel_map(None);
                for (ch, a) in chans {
                    if self.objects.get(a).is_none_or(|o| o.is_none()) || !self.is_a(a, "Pawn") {
                        continue;
                    }
                    let Some(ctrl) = self.native_object(a, crate::NativeField::Controller) else { continue };
                    if self.is_a(ctrl, "PlayerController") && self.native_object(ctrl, crate::NativeField::Player).is_some() {
                        self.net_replicate_actor(None, ch, &chmap);
                    }
                }
            }
            // Connection closed: SetProgress(ConnectionFailed, "", 8) and ReportNetFailure every
            // frame.
            ConnState::Closed => self.net_connection_failed(8.0),
            _ => {}
        }
    }

    /// ULevel::TickNetServer.
    pub(crate) fn net_tick_server(&mut self, dt: f32) {
        if !self.net_is_server() {
            return;
        }
        let first_net = self.net_first_relevant();
        let candidates: Vec<ObjId> = self.actors.iter().take(2).chain(self.actors.iter().skip(first_net)).copied().collect();
        for a in candidates {
            if self.objects.get(a).is_none_or(|o| o.is_none()) || !self.propr(a, "bNetDirty").as_bool() {
                continue;
            }
            let n = self.ipdrv.net_driver.as_ref().map_or(0, |d| d.conns.len());
            for i in (0..n).rev() {
                let Some(c) = self.net_conn(Some(i)) else { continue };
                if c.pc.is_some() && c.conn.state == ConnState::Open {
                    if let Some(&ch) = c.actor_map.get(&a) {
                        if let Some(ac) = c.actor_channels.get_mut(&ch) {
                            ac.actor_dirty = true;
                        }
                    }
                }
            }
            self.set_prop(a, "bNetDirty", Value::Bool(false));
        }
        let n = self.ipdrv.net_driver.as_ref().map_or(0, |d| d.conns.len());
        for i in (0..n).rev() {
            self.net_server_tick_client(i);
        }
        // Mods build only: skins messages of the actors whose channels opened (mod_net_skins.rs).
        #[cfg(feature = "mods")]
        self.mod_net_send_skins();
        // Mods build only: the effects of the scripts (mod_net_fx.rs).
        #[cfg(feature = "mods")]
        self.mod_net_send_fx();
        // Once per second of TimeSeconds (integer part of TimeSeconds − DeltaSeconds, fsubr rounded to 24 bits, ≠ that of TimeSeconds):
        // NAME_Title "<Title> : <map> (<n> players)" (LocalizeProgress RunningNet). The statistics messages
        // of the clients (UserFlags 1 and 2, rdtsc durations) are not reproduced: they depend on the machine's clock.
        let diff = self.level_time - dt as f64;
        let before_value = if crate::net::fpu53() { diff as i32 } else { diff as f32 as i32 };
        if before_value != self.level_time as i32 {
            let fmt = self.localize("Progress", "RunningNet", "Engine", false);
            let title = self.actors.first().map(|&l| self.propr(l, "Title").as_name()).unwrap_or_default();
            let text = fmt.replacen("%s", &title, 1).replacen("%s", &self.url.map.clone(), 1).replacen("%i", &n.to_string(), 1);
            self.log.push(format!("Title: {text}"));
        }
    }

    /// ULevel::iFirstNetRelevantActor (+0x104): after the first two, the bStatic actors without
    /// bAlwaysRelevant (LoadMap, sort_actor_list).
    fn net_first_relevant(&self) -> usize {
        let mut i = 2.min(self.actors.len());
        while let Some(&a) = self.actors.get(i) {
            if !(self.propr(a, "bStatic").as_bool() && !self.propr(a, "bAlwaysRelevant").as_bool()) {
                break;
            }
            i += 1;
        }
        i
    }

    /// ULevel::ServerTickClient. In tests, the outgoing calls can be stubbed ("stc_*").
    pub(crate) fn net_server_tick_client(&mut self, i: usize) {
        let which = Some(i);
        let Some((pc, state, last_recv)) = self.net_conn_ref(which).map(|c| (c.pc, c.conn.state, c.conn.last_receive_time)) else { return };
        let time = self.net_time();
        let Some(pc) = pc.filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some())) else { return };
        if !self.stc_conn_ready(which) || state != ConnState::Open || !(fpu_sub(time, last_recv) < 1.5) {
            return;
        }
        let net_tag = {
            let t = &mut self.ipdrv.rep.net_tag;
            *t += 1;
            *t
        };
        let tick = {
            let c = self.net_conn(which).unwrap();
            c.tick_count += 1;
            for &a in &c.sent_temporaries.clone() {
                let _ = a;
            }
            c.tick_count
        };
        let time_value: Vec<ObjId> = self.net_conn_ref(which).map(|c| c.sent_temporaries.clone()).unwrap_or_default();
        for a in time_value {
            if self.objects.get(a).is_some_and(|o| o.is_some()) {
                self.set_prop(a, "NetTag", Value::Int(net_tag));
            }
        }
        // Viewpoint: ViewTarget and PlayerCalcView.
        let mut viewer = self.po(pc, "ViewTarget").unwrap_or(pc);
        let mut location = self.propr(viewer, "Location").as_vector();
        let mut rotation = self.propr(pc, "Rotation").as_rotator();
        if let Some(f) = self.find_function(pc, "PlayerCalcView", false) {
            let args = vec![Some(Value::Object(Some(viewer))), Some(Value::vector(location[0], location[1], location[2])), Some(Value::rotator(rotation[0], rotation[1], rotation[2]))];
            if let Ok((_, outs)) = self.call(f, pc, args) {
                if let Some(Some(Value::Object(Some(v)))) = outs.first() {
                    viewer = *v;
                }
                if let Some(Some(v)) = outs.get(1) {
                    location = v.as_vector();
                }
                if let Some(Some(v)) = outs.get(2) {
                    rotation = v.as_rotator();
                }
            }
        }
        let _ = rotation;
        // Prediction every other time (0.4 s or 0.9 s), limited by the BSP.
        if tick & 1 != 0 {
            let secs: f32 = if tick & 2 != 0 { 0.4 } else { 0.9 };
            // Velocity × secs as a float; with a Base, the Base's Z and Y go through
            // a float, X stays in the register (fmul then fadd), and the Z sum keeps the FPU's precision
            // until Location.Z. At 24 bits, each operation rounds like a float.
            let mut ahead = self.propr(viewer, "Velocity").as_vector().map(|x| x * secs);
            let fpu53 = crate::net::fpu53();
            let mut z_reg = ahead[2] as f64;
            if let Some(b) = self.native_object(viewer, crate::NativeField::Base) {
                let bv = self.propr(b, "Velocity").as_vector();
                let (bz, by) = (secs * bv[2], secs * bv[1]);
                if fpu53 {
                    ahead[0] = (secs as f64 * bv[0] as f64 + ahead[0] as f64) as f32;
                    ahead[1] = (ahead[1] as f64 + by as f64) as f32;
                    z_reg = ahead[2] as f64 + bz as f64;
                } else {
                    ahead[0] += secs * bv[0];
                    ahead[1] += by;
                    ahead[2] += bz;
                    z_reg = ahead[2] as f64;
                }
            }
            let end = if fpu53 {
                [(location[0] as f64 + ahead[0] as f64) as f32, (location[1] as f64 + ahead[1] as f64) as f32, (z_reg + location[2] as f64) as f32]
            } else {
                [location[0] + ahead[0], location[1] + ahead[1], location[2] + ahead[2]]
            };
            {
                location = self.net_bsp_line_end(end, location);
            }
        }
        let view_pos = self.propr(viewer, "Location").as_vector();
        // ViewDir: Rotation of the controller (+0xD8), not the one returned by PlayerCalcView.
        let view_dir = crate::ai_move::rotator_vector(self.propr(pc, "Rotation").as_rotator());
        let last_rep = self.net_conn_ref(which).map_or(0.0, |c| c.last_rep_time);
        let max_tick = self.net_max_tick_rate();
        let mut lr = last_rep;
        let mut tt = time;
        let mut list: Vec<(i32, ObjId, Option<usize>)> = Vec::new();
        let actors_n = self.actors.len();
        let first_net = self.net_first_relevant();
        let indices: Vec<usize> = (0..2.min(actors_n)).chain(first_net..actors_n).collect();
        for (k, &ai) in indices.iter().enumerate() {
            let Some(&a) = self.actors.get(ai) else { continue };
            if self.objects.get(a).is_none_or(|o| o.is_none()) || self.propr(a, "NetTag").as_int() == net_tag || self.propr(a, "RemoteRole").as_byte() == 0 {
                continue;
            }
            let always = self.propr(a, "bAlwaysRelevant").as_bool();
            let freq = self.propr(a, "NetUpdateFrequency").as_float();
            let ch = self.net_conn_ref(which).and_then(|c| c.actor_map.get(&a).copied());
            let gated = |vm: &mut Vm, lr: f64, tt: f64| -> bool {
                let x = fpu_mul(freq, lr).round_ties_even() as i32;
                let y = fpu_mul(freq, tt).round_ties_even() as i32;
                if x == y {
                    return false;
                }
                // bOnlyDirtyReplication: nothing to send, just RelevantTime.
                if vm.propr(a, "bOnlyDirtyReplication").as_bool() {
                    if let Some(c) = ch.and_then(|ch| vm.net_conn(which).and_then(|c| c.actor_channels.get_mut(&ch))) {
                        if !c.recent.is_empty() && c.dirty.is_empty() && !c.actor_dirty {
                            c.relevant_time = time;
                            return false;
                        }
                    }
                }
                true
            };
            // fadd qword (double)(float)0.023 with the FPU at 24 bits (exact at 53).
            let fpu53 = crate::net::fpu53();
            let step = |x: &mut f64| *x = if fpu53 { *x + 0.023f32 as f64 } else { fadd_d24(*x, 0.023f32 as f64) as f64 };
            let add = if k < 2 {
                if !always {
                    true
                } else {
                    let g = gated(self, lr, tt);
                    step(&mut lr);
                    step(&mut tt);
                    g
                }
            } else if always {
                let g = gated(self, lr, tt);
                step(&mut lr);
                step(&mut tt);
                g
            } else {
                // bNetOptional excluded: jump, without the two + 0.023 (fcomp / jp:
                // Default.LifeSpan − 0.15 ≥ LifeSpan or NaN).
                if self.propr(a, "bNetOptional").as_bool() {
                    let def = self.objects[self.classes[self.objects[a].as_ref().unwrap().class].cdo].as_ref().map_or(0.0, |o| self.prop_of(o, "LifeSpan"));
                    // fsub dword 0.15 then fcomp dword: at 53 bits the difference is not rounded to float.
                    let life = self.propr(a, "LifeSpan").as_float();
                    let kept = if fpu53 { (def as f64 - 0.15f32 as f64) < life as f64 } else { def - 0.15 < life };
                    if !kept {
                        continue;
                    }
                }
                // fcomp / test ah, 1: NetUpdateFrequency < MaxTickRate or NaN.
                let mut ok = true;
                if !(freq >= max_tick) {
                    let x = fpu_mul(freq, lr).round_ties_even() as i32;
                    let y = fpu_mul(freq, tt).round_ties_even() as i32;
                    ok = x != y;
                }
                step(&mut lr);
                step(&mut tt);
                ok
            };
            if !add {
                continue;
            }
            self.set_prop(a, "NetTag", Value::Int(net_tag));
            let prio = self.net_priority(which, a, ch, view_pos, view_dir, time);
            list.push((prio, a, ch));
        }
        if let Some(c) = self.net_conn(which) {
            c.last_rep_time = time;
        }
        // Sort: decreasing priority, unstable sort.
        crate::net_tri::tri_priorities(&mut list, |x| x.0);
        let relevant_timeout = self.net_drv_cfg().relevant_timeout as f64;
        let mut chmap = self.net_channel_map(which);
        for (_, a, ch) in list {
            if self.objects.get(a).is_none_or(|o| o.is_none()) {
                continue;
            }
            if ch.is_some_and(|ch| self.net_conn_ref(which).and_then(|c| c.actor_channels.get(&ch)).is_none_or(|ac| ac.actor.is_none())) {
                continue;
            }
            let relevant_time = ch.and_then(|ch| self.net_conn_ref(which).and_then(|c| c.actor_channels.get(&ch)).map(|ac| ac.relevant_time));
            let mut can_see = false;
            let check = !self.propr(a, "bTearOff").as_bool() && relevant_time.is_none_or(|rt| fpu_sub(time, rt) > 1.0);
            if check {
                can_see = self.net_is_relevant_for(a, pc, viewer, location);
            }
            if !(can_see || relevant_time.is_some_and(|rt| fpu_sub(time, rt) < relevant_timeout)) {
                if let Some(ch) = ch {
                    {
                        self.net_close_actor_channel(which, ch);
                    }
                    chmap.remove(&a);
                }
                continue;
            }
            let ch = match ch {
                Some(ch) => ch,
                None => {
                    let class = self.objects[a].as_ref().unwrap().class;
                    let index = self.net_conn_ref(which).and_then(|c| c.index.clone());
                    let without_index = index.is_none_or(|ix| self.class_net_index(&ix, class).is_none());
                    if without_index {
                        continue;
                    }
                    let fresh_list_new = self.net_conn(which).and_then(|c| c.conn.create_channel(CHTYPE_ACTOR, true, None));
                    let Some(new) = fresh_list_new else { continue };
                    {
                        self.net_set_channel_actor(which, new, a);
                    }
                    chmap.insert(a, (new, false));
                    new
                }
            };
            if !self.stc_conn_ready(which) {
                break;
            }
            if can_see {
                let f = self.rand();
                if let Some(ac) = self.net_conn(which).and_then(|c| c.actor_channels.get_mut(&ch)) {
                    // RelevantTime = appFrand() × 0.5 + Time (fadd qword, fstp qword): rounded to the FPU's precision.
                    ac.relevant_time = if crate::net::fpu53() { (f * 0.5) as f64 + time } else { fadd_d24((f * 0.5) as f64, time) as f64 };
                }
            }
            let ready = self.net_channel_ready(which, ch);
            if ready {
                {
                    self.net_replicate_actor(which, ch, &chmap);
                }
            }
            if !self.stc_conn_ready(which) {
                break;
            }
        }
    }

    /// UNetConnection::IsNetReady(0) (vtable +0x94) called by ServerTickClient.
    fn stc_conn_ready(&mut self, which: Which) -> bool {
        self.net_is_ready(which, false)
    }

    /// (FActorPriority): priority × 65536 depending on the view direction and the time
    /// since the last send (AActor / APawn::GetNetPriority).
    fn net_priority(&mut self, which: Which, a: ObjId, ch: Option<usize>, view_pos: [f32; 3], view_dir: [f32; 3], time: f64) -> i32 {
        let since = match ch.and_then(|ch| self.net_conn_ref(which).and_then(|c| c.actor_channels.get(&ch))) {
            Some(ac) => (time - ac.last_update_time) as f32,
            None => self.net_drv_cfg().spawn_priority_seconds,
        };
        let mut dot = 0.0f32;
        if !self.propr(a, "bAlwaysRelevant").as_bool() {
            // An actor of the player's pawn in the chain of owners: product 1, the rest is skipped
            // (jumps).
            let pawn = self.net_conn_ref(which).and_then(|c| c.pc).and_then(|pc| self.native_object(pc, crate::NativeField::Pawn));
            let mut owns = false;
            if let Some(pawn) = pawn {
                let mut x = Some(a);
                while let Some(o) = x {
                    if o == pawn {
                        owns = true;
                        break;
                    }
                    x = self.native_object(o, crate::NativeField::Owner);
                }
            }
            if owns {
                dot = 1.0;
            } else if self.propr(a, "RemoteRole").as_byte() > 1 || self.propr(a, "Physics").as_byte() == 0 {
                let l = self.propr(a, "Location").as_vector();
                let d = [l[0] - view_pos[0], l[1] - view_pos[1], l[2] - view_pos[2]];
                // The products are added in the order Y, Z then X; a NaN
                // (fcomp, test ah, 5; jp) goes through the normalization.
                if !((d[1] * view_dir[1] + d[2] * view_dir[2]) + d[0] * view_dir[0] < 0.0) {
                    let n = crate::physics::safe_normal(d);
                    dot = (n[2] * view_dir[2] + n[1] * view_dir[1]) + n[0] * view_dir[0];
                } else {
                    dot = -1.0;
                }
            }
        }
        let lag = self.net_conn_ref(which).map_or(9999.0, |c| c.conn.best_lag);
        let recent = ch.and_then(|ch| self.net_conn_ref(which).and_then(|c| c.actor_channels.get(&ch))).filter(|ac| !ac.recent.is_empty()).map(|ac| ac.recent.clone());
        let p = self.net_get_priority(a, recent.as_deref(), since, lag);
        // fistp: out of range or NaN, the undefined integer (i32::MIN); then
        // add [eax], 0xFFD23940 (bNetOptional, -3 000 000) which wraps around.
        let v = ((dot + 3.0) * p * 65536.0).round_ties_even();
        let prio = if v.is_nan() || !(-2147483648.0..2147483648.0).contains(&v) { i32::MIN } else { v as i32 };
        if self.propr(a, "bNetOptional").as_bool() {
            return prio.wrapping_sub(3_000_000);
        }
        prio
    }

    /// AActor::GetNetPriority; APawn::GetNetPriority: pawn of a walking player
    /// with the same weapon and the same bHidden as the sent state (Sent = Recent): time counts
    /// half, plus twice the predicted difference (Location + Velocity × Lag/2 versus Sent.Location +
    /// Sent.Velocity × (Time + Lag/2)) relative to GroundSpeed.
    fn net_get_priority(&self, a: ObjId, sent: Option<&[Value]>, mut time: f32, lag: f32) -> f32 {
        let np = self.propr(a, "NetPriority").as_float();
        if self.is_a(a, "Pawn") {
            let player = self.native_object(a, crate::NativeField::Controller).is_some_and(|c| self.propr(c, "bIsPlayer").as_bool());
            let slot = |n: &str| self.prop_slot(a, n).ok();
            let sv = |n: &str| sent.and_then(|s| slot(n).and_then(|i| s.get(i)).cloned()).unwrap_or(Value::Void);
            if let Some(_) = sent.filter(|_| player) {
                if !sv("bNetOwner").as_bool() && self.propr(a, "Weapon") == &sv("Weapon") && self.propr(a, "bHidden").as_bool() == sv("bHidden").as_bool() && self.propr(a, "Physics").as_byte() == 1 {
                    let half = lag * 0.5;
                    let (l, v) = (self.propr(a, "Location").as_vector(), self.propr(a, "Velocity").as_vector());
                    let (sl, svv) = (sv("Location").as_vector(), sv("Velocity").as_vector());
                    let mut d2 = 0.0f32;
                    for k in 0..3 {
                        let x = (l[k] + v[k] * half) - (sl[k] + svv[k] * (time + half));
                        d2 += x * x;
                    }
                    // (Difference ÷ GroundSpeed) × 2 + Time × 0.5, a single addition.
                    let f = d2.sqrt() / self.propr(a, "GroundSpeed").as_float();
                    time = (f + f) + time * 0.5;
                }
            }
            return time * np;
        }
        // AActor::GetNetPriority, bAlwaysRelevant: Min(NetUpdateFrequency × 0.1, 1) (fcom / jnp:
        // above 1 or NaN, 1), × NetPriority, × Time.
        if self.propr(a, "bAlwaysRelevant").as_bool() {
            let mut f = self.propr(a, "NetUpdateFrequency").as_float() * 0.1;
            if !(f <= 1.0) {
                f = 1.0;
            }
            return (f * np) * time;
        }
        time * np
    }

    /// UNetConnection::UpdateNetSpeedAccordingToLag: on the client, its rate
    /// (CONFClient*); on the host, the driver's (+0xA8) from the worst BestLag of the connections
    /// (CONFDedicatedServer* if the level of the connection's controller is in
    /// NM_DedicatedServer, otherwise CONFListenServer*). Rate clamped first; above
    /// MaxAllowedLag: the minimum; above LagToTakeAction: interpolation from the maximum towards the
    /// minimum (24-bit FPU, _ftol); otherwise + StepUp (÷ number of connections on the host).
    pub(crate) fn net_update_speed_for_lag(&mut self, which: Which) {
        // Not in the original: enhanced network play skips the lags of a connection's first
        // periods (9999 s before any measurement, then hundreds of seconds of an average over
        // nothing, measured): taken as a dead link, they dropped every rate to its minimum,
        // which it then left at its step (20 to 50 bytes/s a second), minutes after the start.
        let enhanced = self.net_enhanced();
        // Not in the original: an enhanced LAN client in a level (its NETSPEED already sent)
        // sends at ENHANCED_LAN_CLIENT_RATE.
        let lan_client_rate = self.net_enhanced() && self.ipdrv.pending.is_none();
        let Some(d) = self.ipdrv.net_driver.as_ref() else { return };
        let (cfg, lan) = (d.cfg.clone(), d.lan);
        let interpolate = |lag: f32, c: &crate::ipdrv::SpeedCfg, min: i32, max: i32| -> i32 {
            ((lag - c.lag_to_take_action) * (min - max) as f32 / (c.max_allowed_lag - c.lag_to_take_action) + max as f32) as i32
        };
        if which.is_none() {
            let c = cfg.client;
            let (min, max) = if lan && lan_client_rate {
                (c.lan_min.max(crate::ipdrv::ENHANCED_LAN_CLIENT_RATE), c.lan_max.max(crate::ipdrv::ENHANCED_LAN_CLIENT_RATE))
            } else if lan {
                (c.lan_min, c.lan_max)
            } else {
                (c.internet_min, c.internet_max)
            };
            let Some(nc) = self.net_conn(None) else { return };
            let conn = &mut nc.conn;
            if max < conn.current_net_speed {
                conn.current_net_speed = max;
            } else if conn.current_net_speed < min {
                conn.current_net_speed = min;
            }
            let lag = conn.best_lag;
            if enhanced && !lag_measured(lag) {
                return;
            }
            if lag > c.max_allowed_lag {
                conn.current_net_speed = min;
            } else if lag > c.lag_to_take_action {
                if conn.current_net_speed > min {
                    conn.current_net_speed = interpolate(lag, &c, min, max);
                }
            } else if conn.current_net_speed < max {
                conn.current_net_speed += c.step_up;
            }
            return;
        }
        let dedicated = self
            .net_conn_ref(which)
            .and_then(|c| c.pc)
            .and_then(|pc| self.native_object(pc, crate::NativeField::Level))
            .is_some_and(|l| self.propr(l, "NetMode").as_byte() == 1);
        let c = if dedicated { cfg.dedicated_server } else { cfg.listen_server };
        let (min, max) = if lan { (c.lan_min, c.lan_max) } else { (c.internet_min, c.internet_max) };
        let Some(d) = self.ipdrv.net_driver.as_mut() else { return };
        if max < d.server_speed {
            d.server_speed = max;
        } else if d.server_speed < min {
            d.server_speed = min;
        }
        let mut worst = 0.0f32;
        let mut measured = 0;
        for k in &d.conns {
            if enhanced && !lag_measured(k.conn.best_lag) {
                continue;
            }
            measured += 1;
            if k.conn.best_lag > worst {
                worst = k.conn.best_lag;
            }
        }
        if enhanced && measured == 0 {
            return;
        }
        if worst > c.max_allowed_lag {
            d.server_speed = min;
        } else if worst > c.lag_to_take_action {
            if d.server_speed > min {
                d.server_speed = interpolate(worst, &c, min, max);
            }
        } else if d.server_speed < max {
            d.server_speed += c.step_up / d.conns.len() as i32;
        }
    }

    /// UGameEngine::GetMaxTickRate: 120 without a driver; client: LanClientMaxTickRate
    /// if its rate (ServerConnection.CurrentNetSpeed) is UPlayer's ConfiguredLanSpeed (default object),
    /// otherwise NetClientMaxTickRate; host without a connection: NetServerMaxTickRate; otherwise the average
    /// rate of the connections (integer division) chooses the Net pair (if it is closer to
    /// ConfiguredInternetSpeed than to ConfiguredLanSpeed) or the Lan pair, then Max − (Max − Min) × (n − 1)
    /// / 3 below 4 connections, Min beyond.
    pub fn net_max_tick_rate(&mut self) -> f32 {
        let Some(d) = self.ipdrv.net_driver.as_ref() else { return 120.0 };
        let client_speed = d.server_conn.as_ref().map(|c| c.conn.current_net_speed);
        let speeds: Vec<i32> = d.conns.iter().map(|c| c.conn.current_net_speed).collect();
        let lan = d.lan;
        let cfg = self.net_drv_cfg();
        let player = |vm: &mut Vm, key: &str| -> i32 { vm.player_default_int(key) };
        let lan_speed = player(self, "ConfiguredLanSpeed");
        if let Some(speed) = client_speed {
            // Not in the original (enhanced network play, LAN client): the original keeps
            // LanClientMaxTickRate only while the client's rate is exactly ConfiguredLanSpeed; its
            // rate steps up by CONFClientNetSpeedStepUp each second on a good link, so after a
            // second a LAN client fell to NetClientMaxTickRate (45) and the host, at 90, saw its
            // turns change every other frame. A LAN client ticks at the rate its host announces
            // (the host's own, shared out between its clients: Vm::net_host_rate_tick), or at the
            // listen server's LAN rate (at least its own) with a host that announces none, so it
            // sends a move for each of the host's frames.
            if lan && self.net_enhanced() {
                return self.lan_client_tick_rate(cfg.lan_client_max_tick_rate.max(cfg.lan_server_max_tick_rate));
            }
            return if speed == lan_speed { cfg.lan_client_max_tick_rate } else { cfg.net_client_max_tick_rate } as f32;
        }
        let n = speeds.len() as i32;
        let rate: i32 = if n == 0 {
            cfg.net_server_max_tick_rate
        } else {
            let avg = speeds.iter().fold(0i32, |a, &s| a.wrapping_add(s)) / n;
            let net_speed = player(self, "ConfiguredInternetSpeed");
            let (max, min) = if avg.wrapping_sub(net_speed).wrapping_abs() < avg.wrapping_sub(lan_speed).wrapping_abs() {
                (cfg.net_server_max_tick_rate, cfg.net_server_min_tick_rate)
            } else {
                (cfg.lan_server_max_tick_rate, cfg.lan_server_min_tick_rate)
            };
            if n < 4 {
                max - (max - min) * (n - 1) / 3
            } else {
                min
            }
        };
        let rate = rate as f32;
        // Not in the original (enhanced network play, LAN host): the host draws at its screen's
        // refresh rate when that is higher (a 144 Hz screen: 144 instead of 90).
        if lan && self.net_enhanced() {
            return self.lan_host_tick_rate(rate);
        }
        rate
    }

    /// AActor / APawn / APlayerController::IsNetRelevantFor.
    fn net_is_relevant_for(&mut self, a: ObjId, real_viewer: ObjId, viewer: ObjId, src: [f32; 3]) -> bool {
        if self.is_a(a, "PlayerController") && a == real_viewer {
            return true;
        }
        if self.propr(a, "bAlwaysRelevant").as_bool() {
            return true;
        }
        let mut x = Some(a);
        while let Some(o) = x {
            if o == viewer || o == real_viewer {
                return true;
            }
            x = self.native_object(o, crate::NativeField::Owner);
        }
        if a == viewer || self.po(a, "Instigator") == Some(viewer) {
            return true;
        }
        let ambient = self.po(a, "AmbientSound").is_some() || matches!(self.propr(a, "AmbientSound"), Value::Asset(_));
        if ambient {
            let d = self.propr(a, "StabilisationDistance").as_float();
            let l = self.propr(a, "Location").as_vector();
            let v = self.propr(viewer, "Location").as_vector();
            let dd = (l[0] - v[0]).powi(2) + (l[1] - v[1]).powi(2) + (l[2] - v[2]).powi(2);
            if dd < d * d {
                return true;
            }
        }
        let is_pawn = self.is_a(a, "Pawn");
        if !is_pawn {
            if let Some(owner) = self.native_object(a, crate::NativeField::Owner) {
                if self.native_object(a, crate::NativeField::Base) == Some(owner) && !self.propr(a, "bOnlyOwnerSee").as_bool() {
                    return self.net_is_relevant_for(owner, real_viewer, viewer, src);
                }
            }
        }
        let hidden = self.propr(a, "bHidden").as_bool();
        let only_owner = self.propr(a, "bOnlyOwnerSee").as_bool();
        if (hidden || only_owner) && !self.propr(a, "bBlockPlayers").as_bool() && !ambient {
            return false;
        }
        // FDistSquared: within 1024 units (1048576), relevant without a
        // trace (fcomp; NaN continues).
        let l = self.propr(a, "Location").as_vector();
        let dist2 = (l[0] - src[0]).powi(2) + (l[1] - src[1]).powi(2) + (l[2] - src[2]).powi(2);
        if dist2 < 1048576.0 {
            return true;
        }
        // Zone of the actor (Region.Zone, +0x98) with bDistanceFog: beyond DistanceFogEnd
        // (appPow(DistanceFogEnd, 2)), not relevant without a trace.
        let zone = match self.prop(a, "Region") {
            Value::Struct(_, f) => f.first().and_then(|z| z.as_object()).filter(|&z| self.alive(z)),
            _ => None,
        };
        if let Some(z) = zone {
            if self.propr(z, "bDistanceFog").as_bool() {
                let end = self.propr(z, "DistanceFogEnd").as_float();
                if dist2 > end * end {
                    return false;
                }
            }
        }
        if self.net_bsp_visible(l, src) {
            return true;
        }
        if !is_pawn {
            return false;
        }
        // APawn: the head, then the four sides only if CollisionRadius > 72
        // (fcomp; NaN continues).
        let r = self.propr(a, "CollisionRadius").as_float();
        let h = self.propr(a, "CollisionHeight").as_float();
        if self.net_bsp_visible([l[0], l[1], l[2] + h], src) {
            return true;
        }
        if r <= 72.0 {
            return false;
        }
        for p in [[l[0], l[1] + r, l[2]], [l[0], l[1] - r, l[2]], [l[0] + r, l[1], l[2]], [l[0] - r, l[1], l[2]]] {
            if self.net_bsp_visible(p, src) {
                return true;
            }
        }
        false
    }

    /// UModel::FastLineCheck: nothing of the BSP between the two points.
    fn net_bsp_visible(&mut self, a: [f32; 3], b: [f32; 3]) -> bool {
        self.level_bsp.as_ref().is_none_or(|bsp| bsp.fast_line_check(a, b))
    }

    /// Model->LineCheck(NF_NotVisBlocking): point reached towards `end` from `start`.
    fn net_bsp_line_end(&mut self, end: [f32; 3], start: [f32; 3]) -> [f32; 3] {
        let Some(bsp) = self.level_bsp.as_ref() else { return end };
        let mut h = openxiii_assets::bsp::BspHit::new(1.0);
        // NF_NotVisBlocking (4): the surfaces that do not block sight are ignored.
        if bsp.line_check(&mut h, None, end, start, [0.0; 3], 4, 0) {
            end
        } else {
            h.location
        }
    }

    /// Client part of LoadMap: non-static actors destroyed, roles swapped.
    pub fn net_client_purge(&mut self) {
        let list = self.actors.clone();
        for a in list {
            if self.objects.get(a).is_none_or(|o| o.is_none()) {
                continue;
            }
            if self.propr(a, "bStatic").as_bool() || self.propr(a, "bNoDelete").as_bool() {
                self.net_swap_roles(a);
            } else {
                let _ = self.destroy_actor_net(a, true);
            }
        }
    }

    fn prop_of(&self, o: &crate::types::Object, name: &str) -> f32 {
        self.classes[o.class].prop_index.get(&name.to_lowercase()).and_then(|&s| o.props.get(s)).map_or(0.0, |v| v.as_float())
    }

    /// Connections of a listening server (remote players), 0 elsewhere.
    pub fn net_remote_count(&self) -> usize {
        self.ipdrv.net_driver.as_ref().map_or(0, |d| d.conns.len())
    }

    /// Rank of an available actor channel (usage check).
    pub fn net_actor_channel_count(&self) -> usize {
        self.ipdrv.net_driver.as_ref().map_or(0, |d| d.conns.iter().chain(d.server_conn.as_ref()).map(|c| c.actor_channels.len()).sum())
    }
}

const _: usize = MAX_CHANNELS;

/// AActor::PostNetReceive: different FName (case-insensitive, None = empty), different AnimRate byte or
/// bAnimLoop bit; AnimFrame and TweenRate alone restart nothing.
pub(crate) fn sim_anim_changed(new: &Value, old: &Value) -> bool {
    let (Value::Struct(_, n), Value::Struct(_, o)) = (new, old) else {
        return false;
    };
    if n.len() < 5 || o.len() < 5 {
        return false;
    }
    let name = |v: &Value| {
        let s = v.as_name();
        if s.eq_ignore_ascii_case("none") { String::new() } else { s.to_ascii_lowercase() }
    };
    name(&n[0]) != name(&o[0]) || n[2].as_int() as u8 != o[2].as_int() as u8 || n[1].as_bool() != o[1].as_bool()
}

/// UActorChannel::Init: RelevantTime = the driver's Time, LastUpdateTime = Time − SpawnPrioritySeconds (fld float, fsubr qword: the exact difference rounded to
/// 24 bits by the game's FPU, then stored as a double).
pub(crate) fn actor_channel_times(time: f64, spawn_priority: f32) -> (f64, f64) {
    let diff = time - spawn_priority as f64;
    (time, if crate::net::fpu53() { diff } else { diff as f32 as f64 })
}

/// UNetConnection::IsNetReady on the driver's QueuedBytes and the bytes of Out: with
/// Saturate, QueuedBytes = −Out.GetNumBytes(); ready if QueuedBytes + Out.GetNumBytes() <= 0.
pub(crate) fn queued_bytes_ready(queued: &mut i32, out_bytes: i32, saturate: bool) -> bool {
    if saturate {
        *queued = -out_bytes;
    }
    *queued + out_bytes < 1
}

/// FADD of two doubles with the FPU at 24 bits: the exact sum rounded only once to float (the double sum
/// then the float only differ halfway between two floats; the exact error of the sum breaks the tie).
pub(crate) fn fadd_d24(a: f64, b: f64) -> f32 {
    let s = a + b;
    let bb = s - a;
    let e = (a - (s - bb)) + (b - bb);
    let r = s as f32;
    let d = s - r as f64;
    if e == 0.0 || d == 0.0 || !r.is_finite() {
        return r;
    }
    let other_one = if d > 0.0 { r.next_up() } else { r.next_down() };
    if s == (r as f64 + other_one as f64) * 0.5 && (e > 0.0) == (d > 0.0) {
        return other_one;
    }
    r
}

/// FMUL of a float by a double then FSTP dword, depending on the FPU's precision (ServerTickClient).
fn fpu_mul(a: f32, b: f64) -> f32 {
    if crate::net::fpu53() {
        (a as f64 * b) as f32
    } else {
        crate::ai_move::fmul_d(a, b)
    }
}

/// FSUB of two doubles, result kept in the register (compared afterwards) depending on the FPU's precision.
fn fpu_sub(a: f64, b: f64) -> f64 {
    if crate::net::fpu53() {
        a - b
    } else {
        fadd_d24(a, -b) as f64
    }
}

/// Enhanced network play: a connection's lag is a measurement (not the 9999 s set before any, nor
/// the average over nothing of its first periods). No real link takes 10 s.
pub(crate) fn lag_measured(lag: f32) -> bool {
    lag < 10.0
}

#[cfg(test)]
mod lan_client_tick_rate_tests {
    use crate::ipdrv::{NetDriver, NetDriverCfg, Sock};
    use crate::net::{ConnState, Connection};
    use crate::net_game::NetConn;

    /// Max tick rate of a client (`client`) or of a host with one connection, whose rate is
    /// `speed`, on a LAN driver or not, with XIII.ini's rates and ConfiguredLanSpeed 10000.
    fn rate(enhanced: bool, lan: bool, client: bool, speed: i32) -> f32 {
        let mut vm = crate::Vm::new();
        let mut config = crate::config::ConfigCache::new(std::env::temp_dir().join("oxiii-lan-client-tick-rate"));
        config.set_string("Engine.Player", "ConfiguredInternetSpeed", "5000", "User");
        config.set_string("Engine.Player", "ConfiguredLanSpeed", "10000", "User");
        vm.config = Some(config);
        let cfg = NetDriverCfg {
            net_server_max_tick_rate: 45,
            net_server_min_tick_rate: 20,
            lan_server_max_tick_rate: 90,
            lan_server_min_tick_rate: 45,
            net_client_max_tick_rate: 45,
            lan_client_max_tick_rate: 60,
            ..Default::default()
        };
        let mut driver = NetDriver::new(Sock(None, 0), 0, lan, cfg);
        let mut conn = Connection::new(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 7777), ConnState::Open, 0.0, 5000);
        conn.current_net_speed = speed;
        if client {
            driver.server_conn = Some(NetConn::new(conn));
        } else {
            driver.conns.push(NetConn::new(conn));
        }
        vm.ipdrv.net_driver = Some(driver);
        crate::net_smooth::set_test_net_enhanced(enhanced);
        let r = vm.net_max_tick_rate();
        crate::net_smooth::set_test_net_enhanced(false);
        r
    }

    #[test]
    fn enhanced_lan_client_ticks_at_the_hosts_lan_rate() {
        // The original: 60 only at exactly ConfiguredLanSpeed, 45 once the rate has stepped up.
        assert_eq!(rate(false, true, true, 10000), 60.0);
        assert_eq!(rate(false, true, true, 10020), 45.0);
        assert_eq!(rate(false, true, true, 11000), 45.0);
        // Enhanced, LAN client: the listen server's LAN rate whatever its own rate (negative
        // control: with the original's rule a client at 10020 gives 45, and this fails).
        for speed in [10000, 10020, 10500, 11000] {
            assert_eq!(rate(true, true, true, speed), 90.0, "enhanced LAN client at {speed}");
        }
        // Enhanced, not a LAN driver (Internet): the original's rule.
        assert_eq!(rate(true, false, true, 10020), 45.0);
        assert_eq!(rate(true, false, true, 10000), 60.0);
        // Hosts: unchanged.
        for enhanced in [false, true] {
            assert_eq!(rate(enhanced, true, false, 15000), 90.0);
            assert_eq!(rate(enhanced, false, false, 5000), 45.0);
        }
    }
}
