use super::{extension, reference_coverage};
use crate::{Document, command_catalog, render};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Scratch(PathBuf);

impl Scratch {
    fn create() -> Result<Self, String> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("atk-mesh-volume-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("candidate.obj"));
        let _ = std::fs::remove_dir(&self.0);
    }
}

pub(super) fn tool() -> Value {
    command_catalog::tool(
        "reconstruct_volume",
        "Generate an editable full-volume mesh candidate from a masked image using local TripoSR inference.",
        json!({
            "type":"object",
            "properties":{
                "image":{"type":"string"},"mask":{"type":"string"},"output_path":{"type":"string"},
                "resolution":{"type":"integer","minimum":64,"maximum":256},
                "aspect":{"enum":["front","front_three_quarter","side","rear","top","bottom"]},
                "source":{"enum":["original","generated"]}
            },
            "required":["image","output_path"],"additionalProperties":false
        }),
    )
}

pub(super) fn run(request: &Value, output: &Path) -> Result<(String, Value), String> {
    let image = input(request, "image", output)?;
    let mask = request
        .get("mask")
        .map(|_| input(request, "mask", output))
        .transpose()?;
    let coverage = reference_coverage(std::slice::from_ref(request))?;
    let resolution = match request.get("resolution") {
        Some(value) => value
            .as_u64()
            .filter(|value| (64..=256).contains(value))
            .ok_or("resolution must be 64..256")?,
        None => 128,
    };
    let root = std::env::var_os("ATK_MESH_TRIPOSR")
        .map(PathBuf::from)
        .ok_or("set ATK_MESH_TRIPOSR to the local TripoSR checkout")?;
    if !root.join("tsr").join("system.py").is_file() {
        return Err(
            "ATK_MESH_TRIPOSR must identify a TripoSR checkout containing tsr/system.py".into(),
        );
    }
    let python = std::env::var_os("ATK_MESH_PYTHON").unwrap_or_else(|| "python".into());
    let scratch = Scratch::create()?;
    let candidate = scratch.0.join("candidate.obj");
    let mut command = Command::new(python);
    command
        .arg("-c")
        .arg(include_str!("triposr.py"))
        .arg(root)
        .arg(image)
        .arg(&candidate)
        .arg(resolution.to_string())
        .env("TORCH_FORCE_WEIGHTS_ONLY_LOAD", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    if let Some(mask) = mask {
        command.arg(mask);
    }
    let status = command
        .status()
        .map_err(|error| format!("could not launch TripoSR Python: {error}"))?;
    if !status.success() {
        return Err(format!(
            "TripoSR inference failed ({status}); see Python stderr"
        ));
    }

    let mut document = Document::empty();
    document.execute(json!({"command":"import_obj","path":candidate}))?;
    let mut edges = HashMap::<_, usize>::new();
    for face in document.mesh.faces.values() {
        for (a, b) in face
            .iter()
            .copied()
            .zip(face.iter().copied().cycle().skip(1))
        {
            *edges.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let boundary = edges.values().filter(|count| **count == 1).count();
    let nonmanifold = edges.values().filter(|count| **count > 2).count();
    let artifact = if extension(output, "obj") {
        let visibility = render::CaptureVisibility::parse(&json!({}), &document.mesh)?;
        document.export_obj(&visibility, None)?
    } else {
        document.execute(json!({"command":"save"}))?.to_string()
    };
    let result = json!({
        "backend":"triposr","resolution":resolution,
        "vertices":document.mesh.vertices.len(),"faces":document.mesh.faces.len(),
        "boundary_edges":boundary,"nonmanifold_edges":nonmanifold,
        "closed_by_edge_incidence":boundary == 0 && nonmanifold == 0,
        "hidden_geometry":"Inferred from the supplied image. Review the side, rear and occluded features.",
        "reference_coverage":coverage
    });
    Ok((artifact, result))
}

fn input(request: &Value, name: &str, output: &Path) -> Result<PathBuf, String> {
    let path = request[name]
        .as_str()
        .ok_or_else(|| format!("{name} must be a PNG path"))?;
    let path = Path::new(path)
        .canonicalize()
        .map_err(|error| format!("{name}: {error}"))?;
    if !path.is_file() || !extension(&path, "png") {
        return Err(format!("{name} must be a PNG file"));
    }
    if output.canonicalize().is_ok_and(|output| output == path) {
        return Err(format!("output must differ from {name}"));
    }
    Ok(path)
}
