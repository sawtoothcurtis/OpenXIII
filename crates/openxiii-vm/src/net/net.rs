//! Network connections of Engine.dll: UNetConnection, UChannel, UControlChannel, FInBunch,
//! FOutBunch (UnConn.cpp, UnChan.cpp, UnBunch.cpp), independent of the VM.
//!
//! Packet: PacketId (ReadInt MAX_PACKETID), then acks (bit 1, PacketId) and bunches
//! (bit 0, bControl [bOpen bClose], bReliable, ChIndex MAX_CHANNELS, ChSequence MAX_CHSEQUENCE if
//! reliable, ChType 8 if bOpen or reliable, size in bits, data), ended by a 1 bit and 0s
//! up to the byte (UNetConnection::FlushNet). The received numbers are relative to
//! the last known one (MakeRelative). UTcpipConnection: MaxPacket 512,
//! PacketOverhead 32.
//!
//! Reception (UNetConnection::ReceivedPacket): each ack or bunch is handled in its
//! turn and what it asks of the engine (NotifyAcceptingChannel, NotifyReceivedText,
//! UActorChannel::ReceivedBunch, destruction of a channel) is returned to it by `next_event` before
//! reading the rest of the packet, like the original's calls in the middle of its loop.
//!
//! Time computations follow the game's 24-bit FPU: each difference of instants
//! (doubles) is rounded to single. Not reimplemented: file channels and downloading
//! (AllowDownloads=False), the internal ack of demos (InternalAck).

use crate::netbits::{BitReader, BitWriter, Mark};
use std::collections::VecDeque;

pub const MAX_PACKETID: u32 = 16384;
pub const MAX_CHANNELS: usize = 1023;
pub const MAX_CHSEQUENCE: u32 = 1024;
pub const MAX_BUNCH_HEADER_BITS: usize = 64;
pub const MAX_PACKET_HEADER_BITS: usize = 16;
pub const MAX_PACKET_TRAILER_BITS: usize = 1;
pub const RELIABLE_BUFFER: usize = 128;

/// Channel types (UChannel::ChannelClasses, filled by the StaticConstructors of UControlChannel,
/// UActorChannel and UFileChannel).
pub const CHTYPE_CONTROL: u32 = 1;
pub const CHTYPE_ACTOR: u32 = 2;
pub const CHTYPE_FILE: u32 = 3;

/// UChannel::IsKnownChannelType: 0 ≤ type < 8 with a registered class.
pub fn is_known_channel_type(t: u32) -> bool {
    matches!(t, CHTYPE_CONTROL | CHTYPE_ACTOR | CHTYPE_FILE)
}

/// EConnectionState.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Invalid = 0,
    Closed = 1,
    Pending = 2,
    Open = 3,
}

/// BestSignedDifference / MakeRelative (UnConn.h): ((Value − Reference − Max/2) & (Max − 1))
/// − Max/2 + Reference.
pub fn make_relative(value: i32, reference: i32, max: i32) -> i32 {
    let diff = ((value - reference + max / 2) & (max - 1)) - max / 2;
    reference + diff
}

thread_local! {
    static FPU53: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// FPU at 53-bit precision (dedicated server: UCC creates no Direct3D device, which sets the
/// game's FPU to 24 bits). Per thread, so that the host and the client of the loop stay separate.
pub fn set_fpu53(on: bool) {
    FPU53.with(|f| f.set(on));
}

pub fn fpu53() -> bool {
    FPU53.with(|f| f.get())
}

/// Difference of two driver instants on the 24-bit FPU (fsub of two doubles).
fn elapsed(now: f64, then: f64) -> f32 {
    (now - then) as f32
}

/// fsub of two doubles kept on the FPU stack: rounded to 24 bits, or exact at 53 bits.
fn elapsed64(now: f64, then: f64) -> f64 {
    if fpu53() {
        now - then
    } else {
        (now - then) as f32 as f64
    }
}

#[derive(Debug, Clone)]
pub struct OutBunch {
    pub data: BitWriter,
    pub ch_index: usize,
    pub ch_type: u32,
    pub ch_sequence: i32,
    pub packet_id: i32,
    pub b_open: bool,
    pub b_close: bool,
    pub b_reliable: bool,
    pub received_ack: bool,
    pub time: f64,
}

impl OutBunch {
    /// FOutBunch(Channel, bClose): FBitWriter(MaxPacket × 8 − 81: bunch and
    /// packet headers, end bit); overflowed from the start if the channel already has
    /// RELIABLE_BUFFER − 1 + bClose pending reliable bunches.
    pub fn new(conn: &Connection, ch_index: usize, b_close: bool) -> Self {
        let ch = conn.channels[ch_index].as_ref();
        let ch_type = ch.map_or(0, |c| c.ch_type);
        let bits = conn.max_packet * 8 - MAX_BUNCH_HEADER_BITS - MAX_PACKET_TRAILER_BITS - MAX_PACKET_HEADER_BITS;
        let mut data = BitWriter::new(bits);
        if ch.is_some_and(|c| c.out_rec.len() >= RELIABLE_BUFFER - 1 + b_close as usize) {
            data.error = true;
        }
        OutBunch { data, ch_index, ch_type, ch_sequence: 0, packet_id: 0, b_open: false, b_close, b_reliable: false, received_ack: false, time: 0.0 }
    }
}

#[derive(Debug, Clone)]
pub struct InBunch {
    pub data: BitReader,
    pub packet_id: i32,
    pub ch_index: usize,
    pub ch_type: u32,
    pub ch_sequence: i32,
    pub b_open: bool,
    pub b_close: bool,
    pub b_reliable: bool,
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub index: usize,
    pub ch_type: u32,
    pub opened_locally: bool,
    pub open_acked: bool,
    pub closing: bool,
    pub open_temporary: bool,
    /// Packet that opened the channel (-1: not yet).
    pub open_packet_id: i32,
    /// Reliable bunches sent and not yet acknowledged, in order (OutRec).
    pub out_rec: Vec<OutBunch>,
    /// Reliable bunches received early, sorted by sequence (InRec).
    pub in_rec: Vec<InBunch>,
}

/// What a received packet asks of the engine, in the original's order.
#[derive(Debug)]
pub enum NetEvent {
    /// Channel opened by the other side: FNetworkNotify::NotifyAcceptingChannel; the answer goes to
    /// `Connection::accept_channel`.
    Accept { ch: usize, ch_type: u32 },
    /// Text of the control channel (NotifyReceivedText).
    Text(String),
    /// Bunch of an actor channel (UActorChannel::ReceivedBunch).
    Bunch(InBunch),
    /// Packet lost for an actor channel (UActorChannel::ReceivedNak, after the resends).
    Nak { ch: usize, packet_id: i32 },
    /// Channel destroyed (UChannel::ConditionalDestroy), with its OpenAcked (+0x2C).
    Destroyed { ch: usize, ch_type: u32, open_acked: bool },
    /// Log line.
    Log(String),
}

/// Where the reading of a packet is (`Connection::begin_packet`).
#[derive(Debug)]
pub struct PacketCursor {
    r: BitReader,
    packet_id: i32,
    queue: VecDeque<NetEvent>,
    step: Step,
    /// ArIsCriticalError of the bunch read from the packet (not of the pending bunches released).
    critical: bool,
}

#[derive(Debug)]
enum Step {
    /// Next ack or bunch.
    Units,
    /// Bunch of a new channel waiting for the engine's answer.
    Accept(InBunch),
    /// Texts of a control channel bunch (UControlChannel::ReceivedBunch).
    Control { ch: usize, data: BitReader, close: bool, first: bool },
    /// After UChannel::ReceivedBunch: closing, then pending bunches.
    After { ch: usize, close: bool },
    /// End of the packet's bunch (InBunches, corrupted data).
    EndBunch,
    /// End of the packet.
    Done,
}

#[derive(Debug)]
pub struct Connection {
    pub state: ConnState,
    pub max_packet: usize,
    pub packet_overhead: usize,
    /// Rate (CurrentNetSpeed, +0x44) and starting rate (+0x108).
    pub current_net_speed: i32,
    pub initial_net_speed: i32,
    pub negotiated_ver: i32,
    pub challenge: i32,
    /// +0x104: correct answer to the CHALLENGE (ULevel::NotifyReceivedText, LOGIN); on
    /// the host, only these connections receive keep-alive packets.
    pub logged_in: bool,
    /// Remote: address "IP:port" (LowLevelGetRemoteAddress).
    pub remote: std::net::SocketAddrV4,
    /// URL.Host (+0x7C): the requested host (client) or the client's IP (host).
    pub url_host: String,
    /// UTcpipConnection +0x3EF0: appSeconds at creation (connection bursts).
    pub connect_time: f64,
    pub out: BitWriter,
    pub in_packet_id: i32,
    pub out_packet_id: i32,
    pub out_ack_packet_id: i32,
    /// OutReliable (+0x1E88) and InReliable (+0x2E84) of each channel.
    pub out_reliable: Vec<i32>,
    pub in_reliable: Vec<i32>,
    pub channels: Vec<Option<Channel>>,
    pub open_channels: Vec<usize>,
    /// Acks to resend (QueuedAcks +0x3E80, ResendAcks +0x3E8C).
    pub queued_acks: Vec<i32>,
    pub resend_acks: Vec<i32>,
    /// LastReceiveTime (+0xE0), LastSendTime (+0xE8), LastTickTime (+0xF0).
    pub last_receive_time: f64,
    pub last_send_time: f64,
    pub last_tick_time: f64,
    /// TimeSensitive (+0x120): a bunch has been put into the current packet.
    pub time_sensitive: bool,
    /// Bunch merging (UChannel::SendBunch): AllowMerge (+0x11C), copy of the last bunch
    /// sent (LastOut, +0x128, FOutBunch()), sequence of its OutRec entry if it is reliable (LastOutBunch,
    /// +0x124), start (LastStart, +0x10C) and end (LastEnd, +0x114; reset by
    /// FlushNet) marks of this bunch in Out.
    pub allow_merge: bool,
    pub last_out: Option<OutBunch>,
    pub last_out_rec: Option<i32>,
    pub last_start: Mark,
    pub last_end: Mark,
    /// Packets ready for LowLevelSend.
    pub outgoing: Vec<Vec<u8>>,
    /// Bytes sent (packet + PacketOverhead) to add to the driver's QueuedBytes (+0x104).
    pub flushed_bytes: i32,
    /// OutLagPacketId (+0xA80) and OutLagTime (+0x280): send time of each packet (256).
    pub out_lag_id: Vec<i32>,
    pub out_lag_time: Vec<f64>,
    /// Statistics of the period (StatPeriod +0x1A4 = 1 s, StatUpdateTime +0x19C): InBytes
    /// (+0x1F8), OutBytes (+0x1FC), InPackets (+0x1E8), OutPackets (+0x1EC), InBunches (+0x1F0),
    /// OutBunches (+0x1F4), InLossPackets (+0x1E0), OutLossPackets (+0x1E4), InOrder (+0x200),
    /// OutOrder (+0x204), LagCount (+0x208), LagAcc (+0x1D8), BestLagAcc (+0x1DC).
    pub stat_period: f32,
    pub stat_update_time: f64,
    pub in_bytes: i32,
    pub out_bytes: i32,
    pub in_packets: i32,
    pub out_packets: i32,
    pub in_bunches: i32,
    pub out_bunches: i32,
    pub in_loss_packets: i32,
    pub out_loss_packets: i32,
    pub in_order: i32,
    pub out_order: i32,
    pub lag_count: i32,
    pub lag_acc: f32,
    pub best_lag_acc: f32,
    /// Per-second results (+0x1A8…+0x1CC), InLoss (+0x1C0, read by GETLOSS), OutLoss (+0x1C4),
    /// AvgLag (+0x1D4), BestLag (+0x1D0, read by GETPING), loss periods (+0x20C).
    pub in_rate: f32,
    pub out_rate: f32,
    pub in_packets_rate: f32,
    pub out_packets_rate: f32,
    pub in_bunches_rate: f32,
    pub out_bunches_rate: f32,
    pub in_order_rate: f32,
    pub out_order_rate: f32,
    pub in_loss: f32,
    pub out_loss: f32,
    pub avg_lag: f32,
    pub best_lag: f32,
    pub loss_periods: i32,
    /// Frame duration (FrameTime +0x218, appSeconds), last appSeconds (+0x210), accumulated time
    /// (+0x220), frames counted (+0x230), average duration (+0x228).
    pub frame_time: f64,
    pub last_app_seconds: f64,
    pub cumulative_time: f64,
    pub counted_frames: i32,
    pub average_frame_time: f64,
    /// Current UNetDriver::Time (set at each TickDispatch).
    pub now: f64,
    /// Driver settings: a client's connection (Driver->ServerConnection non-null),
    /// KeepAliveTime (+0x54), ConnectionTimeout (+0x4C), InitialConnectTimeout (+0x50).
    pub on_client: bool,
    pub keep_alive_time: f32,
    /// OpenXIII, co-op campaign over the network only: acknowledgements sent every tick.
    pub prompt_acks: bool,
    /// OpenXIII: least time between two packets sent for acknowledgements alone (prompt_acks).
    pub prompt_ack_interval: f32,
    /// GError->Logf of the connection (fatal error in the original), written by TickFlush.
    pub fatal: Vec<String>,
    pub connection_timeout: f32,
    pub initial_connect_timeout: f32,
}

/// What UNetConnection::Tick asks of the engine.
#[derive(Debug, Default)]
pub struct TickResult {
    /// Statistics of the period computed: UpdateNetSpeedAccordingToLag.
    pub stats: bool,
    /// HUD.bBadConnectionAlert (myHUD of the connection's controller, +0x218 bit 8) at the end of the
    /// period.
    pub bad_connection: Option<bool>,
    pub log: Option<String>,
}

/// Configured rate read by UNetConnection::UNetConnection and
/// UGameEngine::GetMaxTickRate: ConfiguredLanSpeed (+0x4C) of UPlayer's default object if
/// the driver is in LAN (bLAN, +0x108), otherwise ConfiguredInternetSpeed (+0x48).
pub fn speed_configured(lan: bool, lan_speed: i32, internet_speed: i32) -> i32 {
    if lan {
        lan_speed
    } else {
        internet_speed
    }
}

impl Connection {
    /// UNetConnection::UNetConnection(Driver, URL) then UTcpipConnection: MaxPacket 512, PacketOverhead 32, given state, driver time, BestLag,
    /// AvgLag, LagAcc and BestLagAcc at 9999, configured rate (speed_configured; 0: 2600, otherwise
    /// at least 1500).
    pub fn new(remote: std::net::SocketAddrV4, state: ConnState, time: f64, net_speed: i32) -> Self {
        let speed = if net_speed == 0 { 0xA28 } else { net_speed.max(0x5DC) };
        let mut c = Connection {
            state,
            max_packet: 0x200,
            packet_overhead: 0x20,
            current_net_speed: speed,
            initial_net_speed: speed,
            negotiated_ver: 600,
            challenge: 0,
            logged_in: false,
            fatal: Vec::new(),
            remote,
            url_host: remote.ip().to_string(),
            connect_time: 0.0,
            out: BitWriter::new(0),
            in_packet_id: -1,
            out_packet_id: 0,
            out_ack_packet_id: -1,
            out_reliable: vec![0; MAX_CHANNELS],
            in_reliable: vec![0; MAX_CHANNELS],
            channels: (0..MAX_CHANNELS).map(|_| None).collect(),
            open_channels: Vec::new(),
            queued_acks: Vec::new(),
            resend_acks: Vec::new(),
            last_receive_time: time,
            last_send_time: time,
            last_tick_time: time,
            time_sensitive: false,
            allow_merge: false,
            last_out: None,
            last_out_rec: None,
            last_start: Mark::default(),
            last_end: Mark::default(),
            outgoing: Vec::new(),
            flushed_bytes: 0,
            out_lag_id: vec![0; 256],
            out_lag_time: vec![0.0; 256],
            stat_period: 1.0,
            stat_update_time: time,
            in_bytes: 0,
            out_bytes: 0,
            in_packets: 0,
            out_packets: 0,
            in_bunches: 0,
            out_bunches: 0,
            in_loss_packets: 0,
            out_loss_packets: 0,
            in_order: 0,
            out_order: 0,
            lag_count: 0,
            lag_acc: 9999.0,
            best_lag_acc: 9999.0,
            in_rate: 0.0,
            out_rate: 0.0,
            in_packets_rate: 0.0,
            out_packets_rate: 0.0,
            in_bunches_rate: 0.0,
            out_bunches_rate: 0.0,
            in_order_rate: 0.0,
            out_order_rate: 0.0,
            in_loss: 0.0,
            out_loss: 0.0,
            avg_lag: 9999.0,
            best_lag: 9999.0,
            loss_periods: 0,
            frame_time: 0.0,
            last_app_seconds: 0.0,
            cumulative_time: 0.0,
            counted_frames: 0,
            average_frame_time: 0.0,
            now: time,
            on_client: false,
            keep_alive_time: KEEP_ALIVE_TIME,
            prompt_acks: false,
            prompt_ack_interval: 0.0,
            connection_timeout: 15.0,
            initial_connect_timeout: 30.0,
        };
        c.init_out();
        c
    }

    /// UNetConnection::InitOut.
    pub fn init_out(&mut self) {
        self.out = BitWriter::new(self.max_packet * 8);
    }

    /// UNetConnection::CreateChannel: first free index (control: 0, otherwise 1…);
    /// UChannel::Init (UControlChannel::Init): OpenPacketId −1.
    pub fn create_channel(&mut self, ch_type: u32, opened_locally: bool, index: Option<usize>) -> Option<usize> {
        let index = match index {
            Some(i) => i,
            None => {
                let mut i = if ch_type == CHTYPE_CONTROL { 0 } else { 1 };
                while i < MAX_CHANNELS && self.channels[i].is_some() {
                    i += 1;
                }
                if i == MAX_CHANNELS {
                    return None;
                }
                i
            }
        };
        self.channels[index] = Some(Channel {
            index,
            ch_type,
            opened_locally,
            open_acked: false,
            closing: false,
            open_temporary: false,
            open_packet_id: -1,
            out_rec: Vec::new(),
            in_rec: Vec::new(),
        });
        self.open_channels.push(index);
        Some(index)
    }

    /// ConditionalDestroy (UChannel::RouteDestroy), UChannel::Destroy /
    /// UControlChannel::Destroy: the channel leaves Channels and OpenChannels; the engine
    /// does the rest (UActorChannel::Destroy).
    fn destroy_channel(&mut self, ch: usize, queue: &mut VecDeque<NetEvent>) {
        let Some(c) = self.channels[ch].take() else { return };
        self.open_channels.retain(|&x| x != ch);
        queue.push_back(NetEvent::Destroyed { ch, ch_type: c.ch_type, open_acked: c.open_acked });
    }

    /// Channels still open, from last to first, removed (UNetConnection::Destroy):
    /// (ChIndex, ChType, OpenAcked).
    pub fn destroy_channels(&mut self) -> Vec<(usize, u32, bool)> {
        let mut out = Vec::new();
        for i in (0..self.open_channels.len()).rev() {
            let ch = self.open_channels[i];
            if let Some(c) = self.channels[ch].take() {
                out.push((ch, c.ch_type, c.open_acked));
            }
        }
        self.open_channels.clear();
        out
    }

    /// UNetConnection::PreSend: a bunch too large even for a new packet is
    /// a fatal error (GError).
    fn pre_send(&mut self, bits: usize) {
        if self.out.num + 1 + bits > self.max_packet * 8 {
            self.flush_net();
        }
        if self.out.num == 0 {
            self.out.write_int(self.out_packet_id as u32, MAX_PACKETID);
        }
        if self.out.num + bits + 1 > self.max_packet * 8 {
            self.fatal.push(format!("PreSend overflowed: {}+{bits}>{}", self.out.num, self.max_packet * 8));
        }
    }

    /// UNetConnection::PostSend.
    fn post_send(&mut self) {
        if self.out.num == self.max_packet * 8 {
            self.flush_net();
        }
    }

    /// UNetConnection::FlushNet: end bit, byte completed, LowLevelSend. An
    /// empty packet goes out when KeepAliveTime is exceeded, on the client or for a connection that
    /// has passed the CHALLENGE. The acks sent go into ResendAcks.
    pub fn flush_net(&mut self) {
        self.last_end = Mark::default();
        self.time_sensitive = false;
        let keep_alive = (self.on_client || self.logged_in) && (self.keep_alive_time as f64) < elapsed64(self.now, self.last_send_time);
        if self.out.num != 0 || keep_alive {
            if self.out.num == 0 {
                self.pre_send(0);
            }
            self.out.write_bit(true);
            while self.out.num & 7 != 0 {
                self.out.write_bit(false);
            }
            // UTcpipConnection::LowLevelSend: sendto.
            self.outgoing.push(self.out.data().to_vec());
            let slot = (self.out_packet_id & 0xff) as usize;
            self.out_lag_id[slot] = self.out_packet_id;
            self.out_lag_time[slot] = self.now;
            self.out_packets += 1;
            self.out_packet_id += 1;
            self.last_send_time = self.now;
            let bytes = (self.out.num_bytes() + self.packet_overhead) as i32;
            self.flushed_bytes += bytes;
            self.out_bytes += bytes;
            self.init_out();
        }
        let q = std::mem::take(&mut self.queued_acks);
        self.resend_acks.extend(q);
    }

    /// UNetConnection::PurgeAcks: resend of the pending acks.
    fn purge_acks(&mut self) {
        let r = std::mem::take(&mut self.resend_acks);
        for id in r {
            self.send_ack(id, false);
        }
    }

    /// UNetConnection::SendAck.
    pub fn send_ack(&mut self, packet_id: i32, first_time: bool) {
        if first_time {
            self.purge_acks();
            self.queued_acks.push(packet_id);
        }
        self.pre_send(crate::netbits::ceil_log_two(MAX_PACKETID) as usize + 1);
        self.out.write_bit(true);
        self.out.write_int(packet_id as u32, MAX_PACKETID);
        self.allow_merge = false;
        self.post_send();
    }

    /// UNetConnection::SendRawBunch: header then data; returns the PacketId.
    fn send_raw_bunch(&mut self, bunch: &mut OutBunch, allow_merge: bool) -> i32 {
        self.out_bunches += 1;
        self.time_sensitive = true;
        let mut header = BitWriter::new(MAX_BUNCH_HEADER_BITS);
        header.write_bit(false);
        header.write_bit(bunch.b_open || bunch.b_close);
        if bunch.b_open || bunch.b_close {
            header.write_bit(bunch.b_open);
            header.write_bit(bunch.b_close);
        }
        header.write_bit(bunch.b_reliable);
        header.write_int(bunch.ch_index as u32, MAX_CHANNELS as u32);
        if bunch.b_reliable {
            header.write_int(bunch.ch_sequence as u32, MAX_CHSEQUENCE);
        }
        if bunch.b_reliable || bunch.b_open {
            header.write_int(bunch.ch_type, 8);
        }
        header.write_int(bunch.data.num as u32, (self.max_packet * 8) as u32);
        self.pre_send(header.num + bunch.data.num);
        self.allow_merge = allow_merge;
        bunch.packet_id = self.out_packet_id;
        bunch.time = self.now;
        self.last_start = self.out.mark();
        self.out.write_bits(&header.buffer, header.num);
        self.out.write_bits(&bunch.data.buffer, bunch.data.num);
        self.post_send();
        bunch.packet_id
    }

    /// UChannel::SendBunch: opening at the first send of a channel opened on our side;
    /// with `merge`, a bunch of the same channel as LastOut, which ends the current packet, is
    /// appended to it (LastOut sent instead of the old one, removed from Out); reliable sequence, OutRec;
    /// bClose: SetClosingFlag (the engine removes the actor from an actor channel).
    pub fn send_bunch(&mut self, mut bunch: OutBunch, merge: bool) -> i32 {
        let ch = bunch.ch_index;
        let Some(c) = self.channels[ch].as_mut() else { return -1 };
        if c.open_packet_id == -1 && c.opened_locally {
            bunch.b_open = true;
            c.open_temporary = !bunch.b_reliable;
        }
        let mut out_rec = None;
        if merge
            && self.last_out.as_ref().is_some_and(|l| l.ch_index == ch)
            && self.allow_merge
            && self.last_end.num != 0
            && self.last_end.num == self.out.num
            && self.out.num_bytes() + bunch.data.num_bytes() + 9 <= self.max_packet
        {
            let mut last = self.last_out.take().unwrap();
            last.data.write_bits(&bunch.data.buffer, bunch.data.num);
            last.b_reliable |= bunch.b_reliable;
            last.b_open |= bunch.b_open;
            last.b_close |= bunch.b_close;
            out_rec = self.last_out_rec;
            bunch = last;
            self.out.pop(self.last_start);
            self.out_bunches -= 1;
        }
        if bunch.b_reliable {
            if out_rec.is_none() {
                self.out_reliable[ch] += 1;
                bunch.ch_sequence = self.out_reliable[ch];
            }
            self.last_out_rec = Some(bunch.ch_sequence);
        } else {
            self.last_out_rec = None;
        }
        bunch.received_ack = false;
        let id = self.send_raw_bunch(&mut bunch, true);
        let Some(c) = self.channels[ch].as_mut() else { return id };
        if bunch.b_reliable {
            // New entry at the end of OutRec, or replacement of the merged bunch's entry.
            match out_rec.and_then(|seq| c.out_rec.iter().position(|b| b.ch_sequence == seq)) {
                Some(k) => c.out_rec[k] = bunch.clone(),
                None => c.out_rec.push(bunch.clone()),
            }
        }
        if c.open_packet_id == -1 && c.opened_locally {
            c.open_packet_id = id;
        }
        if bunch.b_close {
            // UChannel::SetClosingFlag.
            c.closing = true;
        }
        self.last_out = Some(bunch);
        self.last_end = self.out.mark();
        id
    }

    /// UControlChannel::Serialize / UNetConnection Logf: a text as a reliable bunch.
    pub fn send_text(&mut self, text: &str) -> Option<String> {
        if self.channels[0].as_ref().is_none_or(|c| c.closing) {
            return None;
        }
        let mut b = OutBunch::new(self, 0, false);
        b.b_reliable = true;
        write_fstring(&mut b.data, text);
        if b.data.error {
            return Some("DevNet: Control channel bunch overflowed".into());
        }
        self.send_bunch(b, true);
        None
    }

    /// UTcpipConnection::LowLevelDescribe: "Host IP:port state: State".
    pub fn low_level_describe(&self) -> String {
        let state = match self.state {
            ConnState::Invalid => 0,
            ConnState::Closed => 1,
            ConnState::Pending => 2,
            ConnState::Open => 3,
        };
        low_level_describe_text(&self.url_host, self.remote.ip().octets(), self.remote.port(), state)
    }

    /// UChannel::Describe: "State=open" or "State=closing".
    pub fn channel_state(&self, ch: usize) -> &'static str {
        if self.channels[ch].as_ref().is_some_and(|c| c.closing) {
            "State=closing"
        } else {
            "State=open"
        }
    }

    /// UChannel::IsNetReady: fewer than 127 pending reliable bunches; the connection
    /// then decides (the driver's QueuedBytes).
    /// UChannel::MaxSendBytes: room for a bunch in the current packet, in bytes, at least 0.
    pub fn max_send_bytes(&self) -> usize {
        crate::net_file::max_send_bytes_of(self.max_packet as i32, self.out.num as i32) as usize
    }

    pub fn channel_out_full(&self, ch: usize) -> bool {
        self.channels[ch].as_ref().is_some_and(|c| c.out_rec.len() > 0x7E)
    }

    /// UChannel::Close: reliable closing bunch if the connection is open or
    /// pending.
    pub fn close_channel(&mut self, ch: usize) {
        let Some(c) = self.channels[ch].as_ref() else { return };
        if !c.closing && matches!(self.state, ConnState::Open | ConnState::Pending) {
            let mut b = OutBunch::new(self, ch, true);
            b.b_reliable = true;
            self.send_bunch(b, false);
        }
    }

    /// UChannel::ReceivedNak: resend of this packet's reliable bunches.
    fn channel_received_nak(&mut self, ch: usize, packet_id: i32) {
        let resend: Vec<usize> = self.channels[ch].as_ref().map_or(Vec::new(), |c| {
            c.out_rec.iter().enumerate().filter(|(_, b)| b.packet_id == packet_id && !b.received_ack).map(|(k, _)| k).collect()
        });
        for k in resend {
            let Some(mut b) = self.channels[ch].as_mut().map(|c| c.out_rec[k].clone()) else { continue };
            self.send_raw_bunch(&mut b, false);
            if let Some(c) = self.channels[ch].as_mut() {
                c.out_rec[k] = b;
            }
        }
    }

    /// UNetConnection::ReceivedNak: each channel, from last to first, resends
    /// (UActorChannel::ReceivedNak: the engine marks this packet's unreliable properties
    /// to be sent again), then ReceivedAcks if the lost packet was the opening one.
    fn received_nak(&mut self, packet_id: i32, queue: &mut VecDeque<NetEvent>) {
        for i in (0..self.open_channels.len()).rev() {
            let Some(&ch) = self.open_channels.get(i) else { continue };
            self.channel_received_nak(ch, packet_id);
            let Some(c) = self.channels[ch].as_ref() else { continue };
            if c.ch_type == CHTYPE_ACTOR {
                queue.push_back(NetEvent::Nak { ch, packet_id });
            }
            if c.open_packet_id == packet_id {
                self.received_acks(ch, queue);
            }
        }
    }

    /// UChannel::ReceivedAcks: acknowledged OutRec removed; a closed and
    /// acknowledged channel, or one opened for a single unreliable acknowledged bunch, is destroyed.
    fn received_acks(&mut self, ch: usize, queue: &mut VecDeque<NetEvent>) {
        let Some(c) = self.channels[ch].as_mut() else { return };
        let mut closed = false;
        while c.out_rec.first().is_some_and(|b| b.received_ack) {
            let b = c.out_rec.remove(0);
            closed |= b.b_close;
        }
        if closed || (c.open_temporary && c.open_acked) {
            self.destroy_channel(ch, queue);
        }
    }

    /// UNetConnection::ReceivedRawPacket: InBytes, InPackets, size in bits
    /// up to the end bit, then ReceivedPacket: PacketId, losses, ack. None if the packet is
    /// empty or badly terminated or its header overflows.
    pub fn begin_packet(&mut self, data: &[u8]) -> Option<PacketCursor> {
        self.in_bytes += self.packet_overhead as i32 + data.len() as i32;
        self.in_packets += 1;
        let &last = data.last()?;
        if last == 0 {
            gnull(format_args!("Packet missing trailing 1"));
            return None;
        }
        let mut bits = data.len() * 8;
        let mut b = last;
        loop {
            bits -= 1;
            if b & 0x80 != 0 {
                break;
            }
            b <<= 1;
        }
        let mut r = BitReader::new(data, bits);
        // UNetConnection::ReceivedPacket.
        if r.error {
            gnull(format_args!("Packet too small"));
            return None;
        }
        self.last_receive_time = self.now;
        let packet_id = make_relative(r.read_int(MAX_PACKETID) as i32, self.in_packet_id, MAX_PACKETID as i32);
        if packet_id > self.in_packet_id {
            self.in_loss_packets += packet_id - self.in_packet_id - 1;
            self.in_packet_id = packet_id;
        } else {
            self.in_order += 1;
        }
        self.send_ack(packet_id, true);
        Some(PacketCursor { r, packet_id, queue: VecDeque::new(), step: Step::Units, critical: false })
    }

    /// What the engine must do next for the current packet; None at the end of the packet.
    pub fn next_event(&mut self, cur: &mut PacketCursor) -> Option<NetEvent> {
        loop {
            if let Some(ev) = cur.queue.pop_front() {
                return Some(ev);
            }
            match std::mem::replace(&mut cur.step, Step::Done) {
                Step::Done => return None,
                Step::Accept(b) => {
                    // No answer from the engine: refusal.
                    cur.step = Step::Accept(b);
                    self.accept_channel(cur, false);
                }
                Step::Units => self.read_unit(cur),
                Step::Control { ch, mut data, close, first } => {
                    let s = read_fstring(&mut data);
                    if data.error {
                        if first {
                            cur.critical |= data.critical;
                        }
                        cur.step = Step::After { ch, close };
                    } else {
                        cur.step = Step::Control { ch, data, close, first };
                        return Some(NetEvent::Text(s));
                    }
                }
                Step::After { ch, close } => {
                    if close {
                        // UChannel::ReceivedSequencedBunch: "got close-notify".
                        self.destroy_channel(ch, &mut cur.queue);
                        cur.step = Step::EndBunch;
                        continue;
                    }
                    // UChannel::ReceivedRawBunch: pending bunches whose
                    // sequence follows.
                    let next = self.in_reliable[ch] + 1;
                    let ready = self.channels[ch].as_ref().is_some_and(|c| c.in_rec.first().is_some_and(|b| b.ch_sequence == next));
                    if ready {
                        let b = self.channels[ch].as_mut().unwrap().in_rec.remove(0);
                        self.received_sequenced_bunch(cur, b, false);
                    } else {
                        cur.step = Step::EndBunch;
                    }
                }
                Step::EndBunch => {
                    self.in_bunches += 1;
                    cur.step = Step::Units;
                    if !self.on_client && cur.critical {
                        cur.critical = false;
                        self.state = ConnState::Closed;
                        return Some(NetEvent::Log(format!("DevNet: Received corrupted packet data from client {}.  Disconnecting.", self.remote)));
                    }
                }
            }
        }
    }

    /// Next ack or bunch of the packet (body of the ReceivedPacket loop).
    fn read_unit(&mut self, cur: &mut PacketCursor) {
        let r = &mut cur.r;
        if r.at_end() || self.state == ConnState::Closed {
            return;
        }
        let is_ack = r.read_bit();
        if r.error {
            gnull(format_args!("Bunch missing ack flag"));
            return;
        }
        if is_ack {
            let ack = make_relative(r.read_int(MAX_PACKETID) as i32, self.out_ack_packet_id, MAX_PACKETID as i32);
            if r.error {
                gnull(format_args!("Bunch missing ack"));
                return;
            }
            if ack > self.out_ack_packet_id {
                for nak in self.out_ack_packet_id + 1..ack {
                    self.received_nak(nak, &mut cur.queue);
                    self.out_loss_packets += 1;
                }
                self.out_ack_packet_id = ack;
            }
            // Round-trip time, minus half the frame, 24-bit FPU.
            let slot = (ack & 0xff) as usize;
            if self.out_lag_id[slot] == ack {
                self.lag_count += 1;
                if fpu53() {
                    self.lag_acc = (((self.now - self.out_lag_time[slot]) - self.frame_time * 0.5) + self.lag_acc as f64) as f32;
                } else {
                    let half = (self.frame_time * 0.5) as f32;
                    self.lag_acc = (elapsed(self.now, self.out_lag_time[slot]) - half) + self.lag_acc;
                }
            }
            for i in (0..self.open_channels.len()).rev() {
                let Some(&ch) = self.open_channels.get(i) else { continue };
                if let Some(c) = self.channels[ch].as_mut() {
                    for b in c.out_rec.iter_mut() {
                        if b.packet_id == ack {
                            b.received_ack = true;
                            if b.b_open {
                                c.open_acked = true;
                            }
                        }
                    }
                    if c.open_packet_id == ack {
                        c.open_acked = true;
                    }
                }
                self.received_acks(ch, &mut cur.queue);
            }
            cur.step = Step::Units;
            return;
        }
        let start = r.pos;
        let b_control = r.read_bit();
        let (b_open, b_close) = if b_control { (r.read_bit(), r.read_bit()) } else { (false, false) };
        let b_reliable = r.read_bit();
        let ch_index = r.read_int(MAX_CHANNELS as u32) as usize;
        let ch_sequence = if b_reliable { make_relative(r.read_int(MAX_CHSEQUENCE) as i32, self.in_reliable[ch_index], MAX_CHSEQUENCE as i32) } else { 0 };
        let ch_type = if b_reliable || b_open { r.read_int(8) } else { 0 };
        let size = r.read_int((self.max_packet * 8) as u32) as usize;
        let header_end = r.pos;
        if r.error {
            gnull(format_args!("Bunch header overflowed"));
            return;
        }
        let mut data = r.sub_reader(size);
        if r.error {
            gnull(format_args!("Bunch data overflowed ({start} {header_end}+{size}/{})", r.num));
            return;
        }
        // FInBunch: ArMaxSerializeSize 10000.
        data.max_serialize = 10000;
        let bunch = InBunch { data, packet_id: cur.packet_id, ch_index, ch_type, ch_sequence, b_open, b_close, b_reliable };
        // Missing channel: only the control channel (0, type 1) can be created if nothing is open.
        if self.channels[ch_index].is_none() && self.channels[0].is_none() && (ch_index != 0 || ch_type != CHTYPE_CONTROL) {
            return;
        }
        cur.step = Step::Units;
        if b_reliable {
            if ch_sequence <= self.in_reliable[ch_index] {
                return;
            }
        } else if !(b_open && b_close) && self.channels[ch_index].as_ref().is_none_or(|c| c.open_packet_id == -1) {
            return;
        }
        if self.channels[ch_index].is_none() {
            if !is_known_channel_type(ch_type) {
                cur.step = Step::Done;
                return;
            }
            self.create_channel(ch_type, false, Some(ch_index));
            cur.queue.push_back(NetEvent::Accept { ch: ch_index, ch_type });
            cur.step = Step::Accept(bunch);
            return;
        }
        self.received_raw_bunch(cur, bunch);
    }

    /// The engine's answer to NetEvent::Accept: accepted, the bunch goes on; refused, reliable
    /// closing bunch, FlushNet, channel destroyed (the connection closes if it is the control channel).
    pub fn accept_channel(&mut self, cur: &mut PacketCursor, accepted: bool) {
        let Step::Accept(bunch) = std::mem::replace(&mut cur.step, Step::Units) else { return };
        let ch = bunch.ch_index;
        if accepted {
            self.received_raw_bunch(cur, bunch);
            return;
        }
        let mut close = OutBunch::new(self, ch, true);
        close.b_reliable = true;
        self.send_bunch(close, false);
        self.flush_net();
        self.destroy_channel(ch, &mut cur.queue);
        if ch == 0 {
            self.state = ConnState::Closed;
        }
    }

    /// Bunch of an existing channel (ReceivedPacket): opening, then
    /// UChannel::ReceivedRawBunch: in reliable order, otherwise put on hold.
    fn received_raw_bunch(&mut self, cur: &mut PacketCursor, bunch: InBunch) {
        let ch = bunch.ch_index;
        if bunch.b_open {
            if let Some(c) = self.channels[ch].as_mut() {
                c.open_acked = true;
                c.open_packet_id = cur.packet_id;
            }
        }
        if bunch.b_reliable && bunch.ch_sequence != self.in_reliable[ch] + 1 {
            if let Some(c) = self.channels[ch].as_mut() {
                let at = c.in_rec.partition_point(|b| b.ch_sequence < bunch.ch_sequence);
                if c.in_rec.get(at).is_none_or(|b| b.ch_sequence != bunch.ch_sequence) && c.in_rec.len() < RELIABLE_BUFFER {
                    c.in_rec.insert(at, bunch);
                }
            }
            cur.step = Step::EndBunch;
            return;
        }
        self.received_sequenced_bunch(cur, bunch, true);
    }

    /// UChannel::ReceivedSequencedBunch: InReliable, then ReceivedBunch if the
    /// channel is not closing; bClose destroys the channel afterwards.
    fn received_sequenced_bunch(&mut self, cur: &mut PacketCursor, bunch: InBunch, first: bool) {
        let ch = bunch.ch_index;
        if bunch.b_reliable {
            self.in_reliable[ch] = bunch.ch_sequence;
        }
        let close = bunch.b_close;
        let Some(c) = self.channels[ch].as_ref() else {
            cur.step = Step::EndBunch;
            return;
        };
        if c.closing {
            cur.step = Step::After { ch, close };
        } else if c.ch_type == CHTYPE_CONTROL {
            cur.step = Step::Control { ch, data: bunch.data, close, first };
        } else {
            cur.step = Step::After { ch, close };
            cur.queue.push_back(NetEvent::Bunch(bunch));
        }
    }

    /// UNetConnection::Tick: frame duration (appSeconds), statistics of the
    /// period, timeout (InitialConnectTimeout while pending or without a controller, otherwise
    /// ConnectionTimeout) or UChannel::Tick of each channel; connection closed if the control
    /// channel has disappeared after use; acks, then FlushNet if a bunch is waiting or
    /// KeepAliveTime is exceeded.
    pub fn tick(&mut self, app_seconds: f64, has_actor: bool) -> TickResult {
        let mut out = TickResult::default();
        self.frame_time = elapsed64(app_seconds, self.last_app_seconds);
        self.last_app_seconds = app_seconds;
        self.cumulative_time = if fpu53() { self.frame_time + self.cumulative_time } else { (self.frame_time as f32 + self.cumulative_time as f32) as f64 };
        self.counted_frames += 1;
        if 1.0 < self.cumulative_time {
            self.average_frame_time = if fpu53() { self.cumulative_time / self.counted_frames as f64 } else { (self.cumulative_time as f32 / self.counted_frames as f32) as f64 };
            self.cumulative_time = 0.0;
            self.counted_frames = 0;
        }
        let real = elapsed64(self.now, self.stat_update_time);
        if (self.stat_period as f64) < real {
            // 1 / RealTime kept on the stack: rounded to 24 bits, or in double at 53 bits.
            let inv = if fpu53() { 1.0 / real } else { (1.0f32 / real as f32) as f64 };
            let rate = |n: i32| if fpu53() { (n as f64 * inv) as f32 } else { n as f32 * inv as f32 };
            self.in_rate = rate(self.in_bytes);
            self.out_rate = rate(self.out_bytes);
            self.in_packets_rate = rate(self.in_packets);
            self.out_packets_rate = rate(self.out_packets);
            self.in_bunches_rate = rate(self.in_bunches);
            self.out_bunches_rate = rate(self.out_bunches);
            self.in_order_rate = rate(self.in_order);
            self.out_order_rate = rate(self.out_order);
            self.out_loss = (self.out_loss_packets as f32 / self.out_packets_rate.max(1.0)) * 100.0;
            self.in_loss = (self.in_loss_packets as f32 / (self.in_loss_packets as f32 + self.in_packets_rate).max(1.0)) * 100.0;
            if self.lag_count != 0 {
                self.avg_lag = self.lag_acc / self.lag_count as f32;
            }
            self.best_lag = self.avg_lag;
            out.stats = true;
            if 20.0 < self.out_loss || 20.0 < self.in_loss {
                self.loss_periods += 1;
            } else {
                self.loss_periods = 0;
            }
            if has_actor {
                out.bad_connection = Some(0.5 < self.best_lag || self.in_packets_rate < 2.0);
            }
            self.lag_acc = 0.0;
            self.best_lag_acc = 9999.0;
            self.in_bytes = 0;
            self.out_bytes = 0;
            self.in_packets = 0;
            self.out_packets = 0;
            self.in_bunches = 0;
            self.out_bunches = 0;
            self.in_loss_packets = 0;
            self.out_loss_packets = 0;
            self.in_order = 0;
            self.out_order = 0;
            self.lag_count = 0;
            self.stat_update_time = self.now;
        }
        self.last_tick_time = self.now;
        let timeout = if self.state == ConnState::Pending || !has_actor { self.initial_connect_timeout } else { self.connection_timeout };
        let waited = elapsed64(self.now, self.last_receive_time);
        if waited <= timeout as f64 {
            for i in (0..self.open_channels.len()).rev() {
                let Some(&ch) = self.open_channels.get(i) else { continue };
                self.channel_tick(ch);
            }
            if self.channels[0].is_none() && (self.out_reliable[0] != 0 || self.in_reliable[0] != 0) {
                self.state = ConnState::Closed;
            }
        } else {
            if self.state != ConnState::Closed {
                out.log = Some(format!("DevNet: Connection timed out after {:.6} seconds ({:.6})", timeout, waited));
            }
            self.state = ConnState::Closed;
        }
        self.purge_acks();
        // Co-op campaign over the network (prompt_acks): the acknowledgements written by
        // PurgeAcks leave at once instead of waiting for the keep-alive (measured on Banque01:
        // a host with little to send acknowledged the guest's moves about 10 times a second, its
        // measured lag rising to 120-480 ms).
        let since_send = elapsed64(self.now, self.last_send_time);
        if self.time_sensitive || (self.prompt_acks && self.out.num != 0 && since_send >= f64::from(self.prompt_ack_interval)) || (self.keep_alive_time as f64) < elapsed64(self.now, self.last_send_time) {
            self.flush_net();
        }
        out
    }

    /// UChannel::Tick (UActorChannel::Tick): the control channel not
    /// yet open resends its reliable bunches not acknowledged for more than a second.
    fn channel_tick(&mut self, ch: usize) {
        let now = self.now;
        let resend: Vec<usize> = self.channels[ch].as_ref().map_or(Vec::new(), |c| {
            if c.index == 0 && !c.open_acked {
                c.out_rec.iter().enumerate().filter(|(_, b)| !b.received_ack && 1.0 < elapsed64(now, b.time)).map(|(k, _)| k).collect()
            } else {
                Vec::new()
            }
        });
        for k in resend {
            let Some(mut b) = self.channels[ch].as_mut().map(|c| c.out_rec[k].clone()) else { continue };
            self.send_raw_bunch(&mut b, false);
            if let Some(c) = self.channels[ch].as_mut() {
                c.out_rec[k] = b;
            }
        }
    }
}

/// [IpDrv.TcpNetDriver] KeepAliveTime.
pub const KEEP_ALIVE_TIME: f32 = 0.2;

/// GNull->Logf: the original's null output, the text is written nowhere.
fn gnull(line: std::fmt::Arguments) {
    let _ = line;
}

/// operator<<(FArchive&, FString&) when writing: FCompactIndex of the number of
/// characters (final zero included; 0 for an empty string), then a one-byte Serialize per
/// character: on overflow, the bytes that fit are written.
pub fn write_fstring(w: &mut BitWriter, s: &str) {
    if s.is_empty() {
        write_compact(w, 0);
        return;
    }
    let bytes: Vec<u8> = s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).chain(std::iter::once(0)).collect();
    write_compact(w, bytes.len() as i32);
    for b in bytes {
        w.write_bytes(&[b]);
    }
}

/// operator<<(FArchive&, FString&) when reading: a count larger than ArMaxSerializeSize sets
/// ArIsError and ArIsCriticalError without reading anything; wide characters (negative count) beyond
/// 255 read as 0x7F; the string stops at the first zero; a single character read, even non-zero, leaves it
/// empty (Num == 1).
pub fn read_fstring(r: &mut BitReader) -> String {
    let n = read_compact(r);
    let count = n.unsigned_abs() as usize;
    if r.max_serialize != 0 && count > r.max_serialize {
        r.error = true;
        r.critical = true;
        return String::new();
    }
    let mut out = String::new();
    let mut ended = false;
    for _ in 0..count {
        let c = if n < 0 {
            let b = r.read_bytes(2);
            let v = u16::from_le_bytes([b[0], b[1]]);
            if v > 0xff { 0x7f } else { v as u8 }
        } else {
            r.read_bytes(1)[0]
        };
        if c == 0 {
            ended = true;
        }
        if !ended {
            out.push(c as char);
        }
    }
    if count == 1 {
        out.clear();
    }
    out
}

/// FCompactIndex: sign 0x80 and continuation 0x40 in the first byte (6 bits), then 7
/// bits per byte with 0x80 for continuation. When reading, the fifth byte is taken whole and
/// the shifts (shl) as well as the sign's neg overflow on 32 bits.
pub fn write_compact(w: &mut BitWriter, v: i32) {
    let a = v.unsigned_abs();
    let mut b0 = (if v < 0 { 0x80 } else { 0 }) | if a < 0x40 { a as u8 } else { (a & 0x3F) as u8 + 0x40 };
    w.write_bytes(&[b0]);
    if b0 & 0x40 != 0 {
        let mut rest = a >> 6;
        loop {
            b0 = if rest > 0x7F { (rest & 0x7F) as u8 + 0x80 } else { rest as u8 };
            w.write_bytes(&[b0]);
            if b0 & 0x80 == 0 {
                break;
            }
            rest >>= 7;
        }
    }
}

pub fn read_compact(r: &mut BitReader) -> i32 {
    let b0 = r.read_bytes(1)[0];
    let mut v: i32 = 0;
    if b0 & 0x40 != 0 {
        let b1 = r.read_bytes(1)[0];
        if b1 & 0x80 != 0 {
            let b2 = r.read_bytes(1)[0];
            if b2 & 0x80 != 0 {
                let b3 = r.read_bytes(1)[0];
                if b3 & 0x80 != 0 {
                    let b4 = r.read_bytes(1)[0];
                    v = b4 as i32;
                }
                v = ((b3 & 0x7F) as i32).wrapping_add(v.wrapping_shl(7));
            }
            v = ((b2 & 0x7F) as i32).wrapping_add(v.wrapping_shl(7));
        }
        v = ((b1 & 0x7F) as i32).wrapping_add(v.wrapping_shl(7));
    }
    v = ((b0 & 0x3F) as i32).wrapping_add(v.wrapping_shl(6));
    if b0 & 0x80 != 0 {
        v.wrapping_neg()
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4};

    #[test]
    fn compact_round_trip() {
        for v in [0, 1, 63, 64, 100, 8191, 8192, -5, 1_000_000, -70000] {
            let mut w = BitWriter::new(64);
            write_compact(&mut w, v);
            let mut r = BitReader::new(&w.buffer, w.num);
            assert_eq!(read_compact(&mut r), v);
        }
    }

    #[test]
    fn empty_fstring_is_count_zero() {
        let mut w = BitWriter::new(64);
        write_fstring(&mut w, "");
        assert_eq!(w.num, 8);
        assert_eq!(w.buffer[0], 0);
    }

    fn texts(conn: &mut Connection, packet: &[u8], accept: bool) -> Vec<String> {
        let mut out = Vec::new();
        let Some(mut cur) = conn.begin_packet(packet) else { return out };
        while let Some(ev) = conn.next_event(&mut cur) {
            match ev {
                NetEvent::Accept { .. } => conn.accept_channel(&mut cur, accept),
                NetEvent::Text(t) => out.push(t),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn hello_reaches_other_side() {
        let a = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1);
        let mut client = Connection::new(a, ConnState::Pending, 0.0, 0);
        client.on_client = true;
        let mut server = Connection::new(a, ConnState::Open, 0.0, 0);
        client.create_channel(CHTYPE_CONTROL, true, Some(0));
        client.send_text("HELLO REVISION=0 MINVER=600 VER=829");
        client.flush_net();
        let mut got = Vec::new();
        for p in std::mem::take(&mut client.outgoing) {
            server.now = 0.01;
            got.extend(texts(&mut server, &p, true));
        }
        assert_eq!(got, vec!["HELLO REVISION=0 MINVER=600 VER=829".to_string()]);
        server.send_text("WELCOME LEVEL=DM_Amos LONE=0");
        server.flush_net();
        let mut got = Vec::new();
        for p in std::mem::take(&mut server.outgoing) {
            client.now = 0.03;
            got.extend(texts(&mut client, &p, true));
        }
        assert_eq!(got, vec!["WELCOME LEVEL=DM_Amos LONE=0".to_string()]);
        assert!(client.channels[0].as_ref().unwrap().out_rec.is_empty());
    }

    #[test]
    fn refused_channel_is_closed() {
        let a = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1);
        let mut client = Connection::new(a, ConnState::Pending, 0.0, 0);
        let mut server = Connection::new(a, ConnState::Open, 0.0, 0);
        client.create_channel(CHTYPE_CONTROL, true, Some(0));
        client.send_text("HELLO");
        client.flush_net();
        let p = client.outgoing.remove(0);
        assert!(texts(&mut server, &p, false).is_empty());
        assert_eq!(server.state, ConnState::Closed);
        assert!(server.channels[0].is_none());
    }
}

/// UTcpipConnection::LowLevelDescribe: "%s %s state: %s" with URL.Host (+0x7C),
/// IpString(RemoteAddr, ntohs(port)) (without ":port" when it is zero) and the state (+0x6C: 1 Closed, 2 Pending, 3 Open,
/// otherwise Invalid).
pub fn low_level_describe_text(host: &str, addr: [u8; 4], port: u16, state: i32) -> String {
    let state_value = match state {
        2 => "Pending",
        3 => "Open",
        1 => "Closed",
        _ => "Invalid",
    };
    format!("{host} {} state: {state_value}", crate::ipdrv::ip_string(addr, i32::from(port)))
}
