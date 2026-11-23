use gpt_mesh::{Mesh, session::Session};
use serde::Serialize;
use serde_json::{Value, json};
use std::{env, fs, io::Write, path::Path, time::Instant};

#[derive(Serialize)]
struct Project {
    version: u64,
    mesh: Mesh,
}

fn write_grid(directory: &Path, triangles: usize) -> Result<(), Box<dyn std::error::Error>> {
    let cells = triangles / 2;
    let mut width = cells.isqrt();
    while !cells.is_multiple_of(width) {
        width -= 1;
    }
    let height = cells / width;
    let vertex = |x: usize, y: usize| (y * (width + 1) + x + 1) as u64;
    let mut mesh = Mesh::default();
    for y in 0..=height {
        for x in 0..=width {
            mesh.vertices.insert(
                vertex(x, y),
                [x as f64 / width as f64, y as f64 / height as f64, 0.0],
            );
        }
    }
    for y in 0..height {
        for x in 0..width {
            let face = 2 * (y * width + x) as u64 + 1;
            mesh.faces.insert(
                face,
                vec![vertex(x, y), vertex(x + 1, y), vertex(x + 1, y + 1)],
            );
            mesh.faces.insert(
                face + 1,
                vec![vertex(x, y), vertex(x + 1, y + 1), vertex(x, y + 1)],
            );
        }
    }
    let file = fs::File::create_new(directory.join("project.json"))?;
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer(&mut writer, &Project { version: 1, mesh })?;
    writer.flush()?;
    Ok(())
}

fn bytes(path: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            total += bytes(&entry.path())?;
        } else {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    let compress = args.get(3).is_some_and(|v| v == "--compress-recordings");
    if args.len() != 3 + usize::from(compress) {
        return Err(
            "usage: bench_dense NEW_DIRECTORY EVEN_TRIANGLE_COUNT [--compress-recordings]".into(),
        );
    }
    let path = Path::new(&args[1]);
    let count: usize = args[2].parse()?;
    if !(100..=1_000_000).contains(&count) || !count.is_multiple_of(2) {
        return Err("choose an even triangle count between 100 and 1000000".into());
    }
    fs::create_dir(path)?;
    write_grid(path, count)?;
    let start = Instant::now();
    let mut session = Session::open_with_recording_compression(path.to_path_buf(), compress)?;
    println!(
        "triangles={count} open_ms={} initial_bytes={}",
        start.elapsed().as_millis(),
        bytes(path)?
    );
    for request in [
        json!({"command":"sculpt_stroke","object":1,"mode":"raise","radius":0.025,"strength":0.01,"samples":[{"face":count / 2 + 1,"barycentric":[0.3,0.3,0.4]}]}),
        json!({"command":"capture","width":256,"height":256,"center":[0.5,0.5,0.0],"zoom":180}),
        json!({"command":"undo"}),
        json!({"command":"save"}),
    ] {
        let start = Instant::now();
        let command = request["command"].as_str().unwrap().to_owned();
        let result: Value = session.execute(request);
        if result["ok"] != true || result.get("recording_error").is_some() {
            return Err(format!(
                "{command}: {} {}",
                result["error"], result["recording_error"]
            )
            .into());
        }
        println!(
            "{command}_ms={} bytes={}",
            start.elapsed().as_millis(),
            bytes(path)?
        );
    }
    drop(session);
    let start = Instant::now();
    let _reopened = Session::open_with_recording_compression(path.to_path_buf(), compress)?;
    println!(
        "reopen_ms={} final_bytes={}",
        start.elapsed().as_millis(),
        bytes(path)?
    );
    Ok(())
}
