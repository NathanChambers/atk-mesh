use crate::workspace_host::saved_workspace_tag;
use std::io::{self, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const HEARTBEAT: Duration = Duration::from_secs(10);
const SAVED_STATE_INTERVAL: Duration = Duration::from_millis(250);

pub enum WorkspaceUpdates {
    Editor,
    Saved { root: PathBuf, workspace: String },
}

#[derive(Default)]
pub struct ViewerUpdates {
    generation: Mutex<u64>,
    changed: Condvar,
}

impl ViewerUpdates {
    pub fn publish(&self) {
        let mut generation = self.generation.lock().unwrap();
        *generation = generation.wrapping_add(1);
        self.changed.notify_all();
    }

    pub fn serve(&self, stream: TcpStream, source: WorkspaceUpdates) -> io::Result<()> {
        stream.set_nodelay(true)?;
        match source {
            WorkspaceUpdates::Editor => self.serve_editor(stream),
            WorkspaceUpdates::Saved { root, workspace } => serve_saved(stream, &root, &workspace),
        }
    }

    fn serve_editor(&self, mut stream: TcpStream) -> io::Result<()> {
        let mut previous = *self.generation.lock().unwrap();
        start_stream(&mut stream)?;
        loop {
            let generation = self.generation.lock().unwrap();
            let (generation, _) = self
                .changed
                .wait_timeout_while(generation, HEARTBEAT, |value| *value == previous)
                .unwrap();
            let current = *generation;
            drop(generation);

            write_update(&mut stream, current != previous)?;
            previous = current;
        }
    }
}

fn serve_saved(mut stream: TcpStream, root: &Path, workspace: &str) -> io::Result<()> {
    let mut previous = saved_workspace_tag(root, Some(workspace)).ok().flatten();
    let mut last_sent = Instant::now();
    start_stream(&mut stream)?;
    loop {
        thread::sleep(SAVED_STATE_INTERVAL);
        let current = saved_workspace_tag(root, Some(workspace)).ok().flatten();
        if current != previous || last_sent.elapsed() >= HEARTBEAT {
            write_update(&mut stream, current != previous)?;
            last_sent = Instant::now();
        }
        previous = current;
    }
}

fn start_stream(stream: &mut TcpStream) -> io::Result<()> {
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n: connected\n\n")
}

fn write_update(stream: &mut TcpStream, changed: bool) -> io::Result<()> {
    if changed {
        stream.write_all(b"event: change\ndata: {}\n\n")
    } else {
        stream.write_all(b": heartbeat\n\n")
    }
}
