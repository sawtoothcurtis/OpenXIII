//! Game messages (`Vm::log`: ScriptLog, Log, traces) to standard output, written by a separate thread. A write
//! to a redirected output or a console costs 0.1 to 0.5 ms (much more on slow machines): when saving a
//! checkpoint or when soldiers raise the alarm, it weighed on the logic frame. The text and its order do not
//! change: each frame sends its lines as one block to the thread, which writes them in arrival order. The thread
//! waits on the channel (no busy loop). `empty` waits until it has written everything (end of the program,
//! before a line written directly that must come after).

use std::io::Write;
use std::sync::mpsc::{channel, Sender};
use std::sync::{LazyLock, Mutex};

enum Message {
    Text(String),
    Empty(Sender<()>),
}

static THREAD: LazyLock<Option<Mutex<Sender<Message>>>> = LazyLock::new(|| {
    let (tx, rx) = channel::<Message>();
    std::thread::Builder::new()
        .name("log".into())
        .spawn(move || {
            while let Ok(m) = rx.recv() {
                match m {
                    Message::Text(t) => write_direct(&t),
                    Message::Empty(ack) => {
                        let _ = ack.send(());
                    }
                }
            }
        })
        .ok()?;
    Some(Mutex::new(tx))
});

fn write_direct(text_value: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text_value.as_bytes());
    let _ = out.flush();
}

/// Lines of `log` (emptied) written as `println!` would, line by line: same text, same order.
pub fn write_value(log: &mut Vec<String>) {
    if log.is_empty() {
        return;
    }
    let mut text_value = String::with_capacity(log.iter().map(|l| l.len() + 1).sum());
    for line in log.drain(..) {
        text_value.push_str(&line);
        text_value.push('\n');
    }
    match THREAD.as_ref() {
        Some(tx) => {
            let refuse = tx.lock().ok().and_then(|tx| tx.send(Message::Text(text_value)).err());
            if let Some(std::sync::mpsc::SendError(Message::Text(t))) = refuse {
                write_direct(&t);
            }
        }
        None => write_direct(&text_value),
    }
}

/// Waits until the thread has written everything sent to it.
pub fn empty() {
    let Some(tx) = THREAD.as_ref() else { return };
    let (ack_tx, ack_rx) = channel();
    if tx.lock().ok().is_some_and(|tx| tx.send(Message::Empty(ack_tx)).is_ok()) {
        let _ = ack_rx.recv_timeout(std::time::Duration::from_secs(5));
    }
}
