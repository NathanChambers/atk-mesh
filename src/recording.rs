use serde_json::{Value, json};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct Recording {
    file: File,
    sequence: usize,
}

impl Recording {
    pub fn create(directory: &Path, session: &str, state: Value) -> Result<Self> {
        fs::create_dir_all(directory)?;
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = directory.join(format!("{timestamp}-{}.jsonl", std::process::id()));
        let file = OpenOptions::new().write(true).create_new(true).open(&path)?;
        let mut recording = Self { file, sequence: 0 };
        recording.write(&json!({"type": "start", "version": 1, "session": session, "state": state}))?;
        eprintln!("Debug recording: {}", path.display());
        Ok(recording)
    }

    pub fn begin(&mut self, request: &Value) -> Result<()> {
        self.sequence += 1;
        self.write(&json!({"type": "request", "sequence": self.sequence, "request": request}))
    }

    pub fn submit_batch(&mut self, requests: &Value) -> Result<()> {
        self.write(&json!({"type": "batch_submission", "requests": requests}))
    }

    pub fn batch_failure(&mut self, error: &str) -> Result<()> {
        self.write(&json!({"type": "batch_failure", "error": error}))
    }

    pub fn finish(&mut self, reply: &Value, state: Value) -> Result<()> {
        let event = json!({"type": "reply", "sequence": self.sequence, "reply": reply, "state": state});
        self.write(&event).map_err(|error| {
            format!("Could not record command outcome: {error}. Check the project before retrying.").into()
        })
    }

    fn write(&mut self, event: &Value) -> Result<()> {
        let line = format!("{event}\n");
        self.file.write_all(line.as_bytes())?;
        self.file.sync_data()?;
        Ok(())
    }
}

pub fn replay(path: &Path, frame: Option<usize>) -> Result<()> {
    replay_to(path, frame, io::stdout().lock())
}

pub fn replay_to(path: &Path, frame: Option<usize>, mut output: impl Write) -> Result<()> {
    let file = File::open(path)?;
    let mut lines = BufReader::new(file).lines();
    let first = lines.next().ok_or("Recording is empty")??;
    let start: Value = serde_json::from_str(&first)?;
    if start["type"] != "start" || start["version"] != 1 {
        return Err("Unsupported debug recording".into());
    }

    if frame == Some(0) {
        if start["state"].is_null() {
            return Err("Recording has no saved snapshot for frame 0".into());
        }
        writeln!(output, "{}", start["state"])?;
        return Ok(());
    }
    if frame.is_none() {
        writeln!(output, "{start}")?;
    }
    for line in lines {
        let line = line?;
        let event: Value = serde_json::from_str(&line)?;
        if let Some(frame) = frame {
            if event["type"] == "reply" && event["sequence"].as_u64() == Some(frame as u64) {
                if event["state"].is_null() {
                    break;
                }
                writeln!(output, "{}", event["state"])?;
                return Ok(());
            }
        } else {
            writeln!(output, "{event}")?;
        }
    }
    if let Some(frame) = frame {
        return Err(format!("Recording has no saved snapshot for frame {frame}").into());
    }

    Ok(())
}
