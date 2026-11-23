use crate::{Document, camera_fit, comparison, objects, projection::Camera};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Guide {
    kind: GuideKind,
    points: Vec<[f64; 2]>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum GuideKind {
    Silhouette,
    Edge,
    Landmark,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Anchor {
    world: [f64; 3],
    pixel: [f64; 2],
}

struct Mask {
    camera: Camera,
    foreground: Vec<bool>,
    weight: f64,
}

pub fn run(request: &Value, output: Option<&str>) -> Result<Option<Value>, String> {
    let Some(command @ ("reference_guides" | "visual_hull")) = request["command"].as_str() else {
        return Ok(None);
    };
    let output = Path::new(output.ok_or("reconstruction commands require --output FILE")?);
    let (artifact, mut result) = match command {
        "reference_guides" => {
            if !extension(output, "svg") && !extension(output, "json") {
                return Err("reference_guides requires an .svg or .json output".into());
            }
            guides(request, extension(output, "svg"))?
        }
        _ => {
            if !extension(output, "obj") && !extension(output, "json") {
                return Err("visual_hull requires an .obj or .json output".into());
            }
            hull(request, extension(output, "obj"))?
        }
    };
    std::fs::write(output, artifact).map_err(|error| format!("{}: {error}", output.display()))?;
    result["path"] = json!(output);
    Ok(Some(result))
}

fn extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn parse_camera(options: &Value) -> Result<Camera, String> {
    if !options.is_object() {
        return Err("each camera must be an object".into());
    }
    Camera::parse(options)
}

fn inside(camera: &Camera, [x, y]: [f64; 2]) -> bool {
    x >= 0.0 && y >= 0.0 && x < camera.width as f64 && y < camera.height as f64
}

fn guides(request: &Value, svg_output: bool) -> Result<(String, Value), String> {
    let initial = parse_camera(&request["camera"])?;
    let fit = if let Some(anchors) = request.get("anchors") {
        let anchors: Vec<Anchor> = serde_json::from_value(anchors.clone())
            .map_err(|error| format!("invalid anchors: {error}"))?;
        if !initial.perspective()
            || anchors.len() > 128
            || anchors.iter().any(|anchor| !inside(&initial, anchor.pixel))
        {
            return Err(
                "camera fitting requires a perspective camera and at most 128 in-frame anchors"
                    .into(),
            );
        }
        let anchors: Vec<_> = anchors
            .into_iter()
            .map(|anchor| (anchor.world, anchor.pixel))
            .collect();
        Some(camera_fit::fit(&initial, &anchors, request)?)
    } else {
        None
    };
    let camera = match &fit {
        Some(fit) => parse_camera(fit)?,
        None => initial,
    };
    let guides: Vec<Guide> = serde_json::from_value(request["guides"].clone())
        .map_err(|error| format!("invalid guides: {error}"))?;
    if guides.is_empty() || guides.len() > 256 {
        return Err("provide 1..256 guides".into());
    }

    let mut output = Vec::with_capacity(guides.len());
    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}" fill="none" stroke-width="2">"#,
        camera.width, camera.height
    );
    for guide in guides {
        let (kind, color, minimum) = match guide.kind {
            GuideKind::Silhouette => ("silhouette", "#00b894", 3),
            GuideKind::Edge => ("edge", "#fd9644", 2),
            GuideKind::Landmark => ("landmark", "#4b7bec", 1),
        };
        if !(minimum..=4096).contains(&guide.points.len())
            || guide.points.iter().any(|pixel| !inside(&camera, *pixel))
        {
            return Err(format!("invalid {kind} point count or out-of-frame pixel"));
        }
        if kind == "landmark" && guide.points.len() != 1 {
            return Err("a landmark must have one point".into());
        }
        let mut rays = Vec::with_capacity(guide.points.len());
        let mut points = String::new();
        for [x, y] in &guide.points {
            let (origin, direction) = camera.ray([*x, *y]);
            if origin
                .iter()
                .chain(&direction)
                .any(|value| !value.is_finite())
            {
                return Err("guide ray exceeds numeric range".into());
            }
            rays.push(json!({"origin":origin,"direction":direction}));
            write!(points, "{x},{y} ").unwrap();
        }
        if kind == "landmark" {
            let [x, y] = guide.points[0];
            writeln!(
                svg,
                r#"<circle data-kind="{kind}" stroke="{color}" cx="{x}" cy="{y}" r="3"/>"#
            )
            .unwrap();
        } else {
            let tag = if kind == "silhouette" {
                "polygon"
            } else {
                "polyline"
            };
            writeln!(
                svg,
                r#"<{tag} data-kind="{kind}" stroke="{color}" points="{points}"/>"#
            )
            .unwrap();
        }
        output.push(json!({"kind":kind,"points":guide.points,"rays":rays}));
    }
    svg.push_str("</svg>\n");
    let mut result = json!({"camera":camera.metadata(),"guides":output});
    if let Some(fit) = fit {
        result["fit_diagnostics"] = fit["fit_diagnostics"].clone();
    }
    let artifact = if svg_output { svg } else { result.to_string() };
    Ok((artifact, result))
}

fn view_list(request: &Value, minimum: usize) -> Result<&[Value], String> {
    request["views"]
        .as_array()
        .filter(|views| (minimum..=32).contains(&views.len()))
        .map(Vec::as_slice)
        .ok_or_else(|| format!("provide {minimum}..32 views"))
}

fn read_png(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut bytes = Vec::new();
    use std::io::Read;
    file.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(format!("{} exceeds 32 MiB", path.display()));
    }
    Ok(bytes)
}

fn hull(request: &Value, obj_output: bool) -> Result<(String, Value), String> {
    let bounds: [[f64; 3]; 2] = serde_json::from_value(request["bounds"].clone())
        .map_err(|_| "bounds must be [[min_x,min_y,min_z],[max_x,max_y,max_z]]")?;
    let span: [f64; 3] = std::array::from_fn(|axis| bounds[1][axis] - bounds[0][axis]);
    if span.iter().any(|value| !value.is_finite() || *value <= 0.0) {
        return Err("bounds must have positive finite spans".into());
    }
    let resolution = match request.get("resolution") {
        Some(value) => value
            .as_u64()
            .filter(|value| (4..=96).contains(value))
            .ok_or("resolution must be 4..96")? as usize,
        None => 40,
    };
    let agreement = match request.get("agreement") {
        Some(value) => value
            .as_f64()
            .filter(|value| *value > 0.0 && *value <= 1.0)
            .ok_or("agreement must be greater than 0 and at most 1")?,
        None => 1.0,
    };
    let mut masks = Vec::new();
    for view in view_list(request, 2)? {
        let camera = parse_camera(&view["camera"])?;
        let path = view["mask"]
            .as_str()
            .ok_or("each view requires a mask PNG path")?;
        let weight = match view.get("weight") {
            Some(value) => value
                .as_f64()
                .filter(|value| *value > 0.0 && *value <= 1.0)
                .ok_or("view weight must be greater than 0 and at most 1")?,
            None => 1.0,
        };
        let bytes = read_png(Path::new(path))?;
        let (width, height, colors) = comparison::decode(&bytes)?;
        if [width, height] != [camera.width, camera.height] {
            return Err(format!("{path}: mask dimensions must match its camera"));
        }
        let white = match view.get("foreground").and_then(Value::as_str) {
            Some("white") => true,
            Some("black") => false,
            None if view.get("foreground").is_none() => true,
            _ => return Err("foreground must be white or black".into()),
        };
        let foreground = colors
            .chunks_exact(3)
            .map(|rgb| (rgb[0] >= 128) == white)
            .collect::<Vec<_>>();
        if !foreground.contains(&true) || !foreground.contains(&false) {
            return Err(format!(
                "{path}: mask must contain both foreground and background"
            ));
        }
        masks.push(Mask {
            camera,
            foreground,
            weight,
        });
    }

    let index = |[x, y, z]: [usize; 3]| (z * resolution + y) * resolution + x;
    let step = span.map(|length| length / resolution as f64);
    let maximum_weight: f64 = masks.iter().map(|mask| mask.weight).sum();
    let mut occupied = vec![false; resolution.pow(3)];
    for z in 0..resolution {
        for y in 0..resolution {
            for x in 0..resolution {
                let cell = [x, y, z];
                let point = std::array::from_fn(|axis| {
                    bounds[0][axis] + (cell[axis] as f64 + 0.5) * step[axis]
                });
                let supported: f64 = masks
                    .iter()
                    .filter(|mask| {
                        let Ok([x, y, _]) = mask.camera.project(point) else {
                            return false;
                        };
                        inside(&mask.camera, [x, y])
                            && mask.foreground[y as usize * mask.camera.width + x as usize]
                    })
                    .map(|mask| mask.weight)
                    .sum();
                occupied[index(cell)] = supported >= agreement * maximum_weight;
            }
        }
    }
    let voxels = occupied.iter().filter(|cell| **cell).count();
    if voxels == 0 {
        return Err(
            "the silhouettes have no shared volume inside bounds at this resolution and agreement"
                .into(),
        );
    }

    let mut vertices = HashMap::new();
    let mut document = Document::empty();
    for z in 0..resolution {
        for y in 0..resolution {
            for x in 0..resolution {
                let cell = [x, y, z];
                if !occupied[index(cell)] {
                    continue;
                }
                for axis in 0..3 {
                    for positive in [false, true] {
                        let mut neighbor = cell;
                        neighbor[axis] = if positive {
                            cell[axis] + 1
                        } else {
                            cell[axis].wrapping_sub(1)
                        };
                        if neighbor[axis] < resolution && occupied[index(neighbor)] {
                            continue;
                        }
                        let corners = if positive {
                            [[0, 0], [1, 0], [1, 1], [0, 1]]
                        } else {
                            [[0, 0], [0, 1], [1, 1], [1, 0]]
                        };
                        let mut face = Vec::with_capacity(4);
                        for [u, v] in corners {
                            let mut corner = cell;
                            corner[axis] += usize::from(positive);
                            corner[(axis + 1) % 3] += u;
                            corner[(axis + 2) % 3] += v;
                            let id = match vertices.entry(corner) {
                                std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
                                std::collections::hash_map::Entry::Vacant(entry) => {
                                    let point = std::array::from_fn(|axis| {
                                        bounds[0][axis] + corner[axis] as f64 * step[axis]
                                    });
                                    *entry.insert(document.mesh.add_vertex(point)?)
                                }
                            };
                            face.push(id);
                        }
                        document.mesh.add_face(face)?;
                    }
                }
            }
        }
    }
    objects::initialize(&mut document.mesh);
    let faces = document.mesh.faces.len();
    let artifact = if obj_output {
        document.export_obj()
    } else {
        document.execute(json!({"command":"save"}))?.to_string()
    };
    let volume_basis = if agreement == 1.0 {
        "The grid retains cells whose centers project inside every supplied silhouette. With accurate masks and cameras, this approximates the largest volume consistent with those silhouettes."
    } else {
        "The grid retains cells whose centers meet the weighted silhouette agreement threshold. It may retain cells outside individual silhouettes and omit parts of the intended object."
    };
    let result = json!({
        "voxels": voxels,
        "vertices": vertices.len(),
        "faces": faces,
        "resolution": resolution,
        "guidance": {
            "stage": "volume_estimate",
            "user_summary": "This first pass estimates the object's outer volume from silhouettes. Use it to check overall proportions before building the main forms and details.",
            "volume_basis": volume_basis,
            "limitations": "Silhouettes leave depth, feature correspondence, and interior details ambiguous. Expect stepped edges, possible duplicate or stretched parts, and missing recesses.",
            "next_steps": [
                "Use the hull to check width, height, and depth. Match landmarks and structural edges across the references before rebuilding the major forms as separate parts.",
                "Use reference_guides for features inside the outline. Estimate their depth from a second view or a known surface, then model them separately.",
                "Check the refined blockout from an oblique angle as well as the input views. Report which proportions have been checked and which features still need work."
            ]
        }
    });
    Ok((artifact, result))
}
