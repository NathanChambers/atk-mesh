use atk_mesh::{Document, base64};
use serde_json::{Value, json};
use std::io::Cursor;
use std::path::Path;
use std::process::Command;

fn invoke(root: &Path, request: Value, output: &str) -> Value {
    let input = root.join("request.json");
    std::fs::write(&input, request.to_string()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_atk-mesh-cli"))
        .current_dir(root)
        .env("MESH_PORT", "invalid")
        .arg("--input")
        .arg(input)
        .args(["--output", output])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let reply: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(reply["ok"], true);
    reply["result"].clone()
}

fn capture(document: &mut Document, mut camera: Value, mode: &str) -> Vec<u8> {
    camera["command"] = json!("capture");
    camera["mode"] = json!(mode);
    let result = document.execute(camera).unwrap();
    base64::decode(result["png_base64"].as_str().unwrap()).unwrap()
}

fn pixels(bytes: Vec<u8>) -> Vec<u8> {
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    pixels
}

#[test]
fn reconstructs_box_and_exports_guides() {
    let root = std::env::temp_dir().join(format!("mesh-reconstruction-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut original = Document::empty();
    original.execute(json!({"command":"object_create_box","name":"Reference box","min":[-0.5,-0.4,-0.3],"max":[0.5,0.4,0.3]})).unwrap();
    let mut views = Vec::new();
    for (name, yaw) in [("front.png", 0), ("side.png", 90)] {
        let camera = json!({"width":160,"height":160,"yaw":yaw,"pitch":0,"zoom":400});
        std::fs::write(
            root.join(name),
            capture(&mut original, camera.clone(), "silhouette"),
        )
        .unwrap();
        views.push(json!({"camera":camera,"mask":name,"foreground":"black"}));
    }
    let hull = invoke(
        &root,
        json!({"command":"visual_hull","views":views,"bounds":[[-1,-1,-1],[1,1,1]],"resolution":40}),
        "hull.json",
    );
    assert_eq!(hull["voxels"], 3840);
    assert_eq!(hull["faces"], 1504);
    assert_eq!(hull["guidance"]["stage"], "volume_estimate");
    assert!(
        hull["guidance"]["user_summary"]
            .as_str()
            .unwrap()
            .contains("outer volume")
    );
    assert!(
        hull["guidance"]["volume_basis"]
            .as_str()
            .unwrap()
            .contains("every supplied silhouette")
    );
    assert!(
        hull["guidance"]["next_steps"][1]
            .as_str()
            .unwrap()
            .contains("reference_guides")
    );
    let project: Value =
        serde_json::from_slice(&std::fs::read(root.join("hull.json")).unwrap()).unwrap();
    let mut reconstructed = Document::empty();
    reconstructed
        .execute(json!({"command":"load","project":project}))
        .unwrap();
    let withheld = json!({"width":160,"height":160,"yaw":34,"pitch":21,"zoom":400});
    let expected = pixels(capture(&mut original, withheld.clone(), "silhouette"));
    let actual = pixels(capture(&mut reconstructed, withheld, "silhouette"));
    let (mut intersection, mut union) = (0, 0);
    for (expected, actual) in expected.chunks_exact(3).zip(actual.chunks_exact(3)) {
        intersection += usize::from(expected[0] == 0 && actual[0] == 0);
        union += usize::from(expected[0] == 0 || actual[0] == 0);
    }
    let iou = intersection as f64 / union as f64;
    println!("Withheld silhouette IoU: {iou:.4}");
    assert!(iou > 0.99);

    let shifted =
        json!({"width":160,"height":160,"yaw":90,"pitch":0,"zoom":400,"center":[0,0.2,0]});
    std::fs::write(
        root.join("shifted.png"),
        capture(&mut original, shifted, "silhouette"),
    )
    .unwrap();
    let mut noisy_views = views.clone();
    let mut noisy_view = noisy_views[1].clone();
    noisy_view["mask"] = json!("shifted.png");
    noisy_view["weight"] = json!(0.25);
    noisy_views.push(noisy_view);
    let mut request = json!({"command":"visual_hull","views":noisy_views,"bounds":[[-1,-1,-1],[1,1,1]],"resolution":40});
    let strict = invoke(&root, request.clone(), "strict.obj");
    request["agreement"] = json!(0.85);
    let tolerant = invoke(&root, request, "tolerant.obj");
    assert_eq!(strict["voxels"], 2880);
    assert_eq!(tolerant["voxels"], hull["voxels"]);
    assert!(
        tolerant["guidance"]["volume_basis"]
            .as_str()
            .unwrap()
            .contains("weighted silhouette agreement")
    );
    println!(
        "Shifted-view case retained {} voxels with strict intersection and {} with weighted agreement",
        strict["voxels"], tolerant["voxels"]
    );

    let camera = json!({"width":160,"height":160,"projection":"perspective","position":[0,0,3],"yaw":0,"pitch":0,"fov":90,"lens_shift":[0.1,0]});
    let guides = json!([
        {"kind":"silhouette","points":[[80,64],[112,64],[112,96],[80,96]]},
        {"kind":"edge","points":[[80,64],[112,64]]},
        {"kind":"landmark","points":[[96,80]]}
    ]);
    let request = json!({"command":"reference_guides","camera":camera,"guides":guides});
    let output = invoke(&root, request, "guides.svg");
    assert_eq!(
        output["guides"][2]["rays"][0]["origin"],
        json!([0.0, 0.0, 3.0])
    );
    let direction = &output["guides"][2]["rays"][0]["direction"];
    assert!(direction[0].as_f64().unwrap().abs() < 1e-12);
    assert_eq!(direction[2], -1.0);
    let svg = std::fs::read_to_string(root.join("guides.svg")).unwrap();
    assert!(svg.contains("<polygon") && svg.contains("<polyline") && svg.contains("<circle"));

    let mut anchors = Vec::new();
    for x in [-0.5, 0.5] {
        for y in [-0.4, 0.4] {
            for z in [-0.3, 0.3] {
                let scale = 80.0 / (3.0 - z);
                anchors.push(json!({"world":[x,y,z],"pixel":[96.0+x*scale,80.0-y*scale]}));
            }
        }
    }
    let mut initial_camera = camera.clone();
    initial_camera["position"] = json!([0.2, 0, 3]);
    let fitted = invoke(
        &root,
        json!({"command":"reference_guides","camera":initial_camera,"guides":guides,"anchors":anchors,"fit_parameters":["position_x"]}),
        "fitted-guides.json",
    );
    assert!(
        fitted["fit_diagnostics"]["rms_canvas_pixels"]
            .as_f64()
            .unwrap()
            < 1e-5
    );

    println!("Test artifacts: {}", root.display());
}
