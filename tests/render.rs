use atk_mesh::{Document, base64};
use serde_json::{Value, json};
use std::io::Cursor;

fn capture(document: &mut Document, mut request: Value) -> (Value, Vec<u8>) {
    request["command"] = json!("capture");
    let result = document.execute(request).unwrap();
    let bytes = base64::decode(result["png_base64"].as_str().unwrap()).unwrap();
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!(json!(info.width), result["width"]);
    assert_eq!(json!(info.height), result["height"]);
    assert_eq!(info.color_type, png::ColorType::Rgb);
    buffer.truncate(info.buffer_size());
    (result, buffer)
}

fn pixel(bytes: &[u8], x: usize, y: usize) -> &[u8] {
    &bytes[(y * 800 + x) * 3..(y * 800 + x) * 3 + 3]
}

#[test]
fn default_capture_is_neutral_grayscale_and_does_not_mutate_geometry_or_history() {
    let mut document = Document::default();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    let (metadata, pixels) = capture(&mut document, json!({"yaw":35,"pitch":25}));
    assert_eq!(
        (metadata["width"].as_u64(), metadata["height"].as_u64()),
        (Some(800), Some(600))
    );
    assert_eq!(metadata["mode"], "shaded");
    assert_eq!(metadata["clean"], true);
    assert!(
        pixels
            .chunks(3)
            .all(|pixel| pixel[0] == pixel[1] && pixel[1] == pixel[2])
    );
    assert_ne!(pixel(&pixels, 400, 300), pixel(&pixels, 0, 0));
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
    let (again, _) = capture(&mut document, json!({"yaw":35,"pitch":25}));
    assert_eq!(again["png_base64"], metadata["png_base64"]);
}

#[test]
fn face_palette_matches_the_browser_and_uses_distinct_saturated_colors() {
    let mut document = Document::default();
    let (metadata, _) = capture(&mut document, json!({"mode":"face_ids"}));
    let colors = [
        [242, 53, 53],
        [53, 108, 242],
        [164, 242, 53],
        [242, 53, 219],
        [53, 242, 211],
        [242, 156, 53],
    ];
    for (index, rgb) in colors.iter().enumerate() {
        let key = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
        assert_eq!(metadata["face_id_colors"][key], index + 1);
    }
}

#[test]
fn object_ids_color_all_faces_by_owner_and_return_exact_color_mapping() {
    let mut document = Document::default();
    document.execute(json!({"command":"object_create_box","name":"Second","min":[1.5,-0.5,-0.5],"max":[2.5,0.5,0.5]})).unwrap();
    let before = document.execute(json!({"command":"save"})).unwrap();
    let (result, image) = capture(
        &mut document,
        json!({"mode":"object_ids","yaw":0,"pitch":0,"center":[1,0,0]}),
    );
    assert_eq!(result["object_id_colors"]["#f23535"], 1);
    assert_eq!(result["object_id_colors"]["#356cf2"], 2);
    assert_eq!(result["face_id_colors"], json!({}));
    let colors: std::collections::BTreeSet<_> = image
        .chunks(3)
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    assert_eq!(
        colors,
        std::collections::BTreeSet::from([[0, 0, 0], [242, 53, 53], [53, 108, 242]])
    );
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
}

#[test]
fn capture_filters_remove_occluders_preserve_colors_and_scope_image_selection() {
    let mut document = Document::default();
    document
        .execute(
            json!({"command":"object_create_box","name":"Cover","min":[-2,-2,2],"max":[2,2,3]}),
        )
        .unwrap();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    let (full, pixels) = capture(
        &mut document,
        json!({"mode":"object_ids","yaw":0,"pitch":0}),
    );
    assert_eq!(pixel(&pixels, 400, 300), [53, 108, 242]);
    let (hidden, pixels) = capture(
        &mut document,
        json!({"mode":"object_ids","hide_objects":[2],"yaw":0,"pitch":0}),
    );
    assert_eq!(pixel(&pixels, 400, 300), [242, 53, 53]);
    assert_eq!(hidden["camera"], full["camera"]);
    assert_eq!(hidden["object_id_colors"], full["object_id_colors"]);
    assert_eq!(hidden["visibility"]["visible_object_count"], 1);
    assert_eq!(hidden["depth_range"], json!([-1.0, 1.0]));
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
    let (_, solo) = capture(
        &mut document,
        json!({"mode":"object_ids","solo_objects":[1],"yaw":0,"pitch":0}),
    );
    assert_eq!(solo, pixels);
    for through in [false, true] {
        let (view, _) = capture(&mut document, json!({"hide_objects":[2],"yaw":0,"pitch":0}));
        let selected = document.execute(json!({"command":"select_image","view_id":view["view_id"],"rect":[0,0,800,600],"through":through})).unwrap();
        let ids = selected["mesh"]["selection"].as_array().unwrap();
        assert_eq!(ids.len(), if through { 8 } else { 4 });
        assert!(ids.iter().all(|id| id.as_u64().unwrap() <= 8));
    }
}

#[test]
fn empty_capture_filters_hide_edges_labels_and_near_plane_crossings_in_every_mode() {
    let mut document = Document::default();
    document
        .execute(
            json!({"command":"object_create_box","name":"Near","min":[-2,-2,4.9],"max":[2,2,5.1]}),
        )
        .unwrap();
    let (_, _) = capture(
        &mut document,
        json!({"projection":"perspective","position":[0,0,5],"yaw":0,"pitch":0,"solo_objects":[1]}),
    );
    for mode in [
        "shaded",
        "topology",
        "normals",
        "depth",
        "silhouette",
        "face_ids",
        "object_ids",
        "orientation",
    ] {
        let (result, pixels) = capture(
            &mut document,
            json!({"mode":mode,"projection":"perspective","position":[0,0,5],"yaw":0,"pitch":0,"solo_objects":[],"clean":mode!="topology"}),
        );
        let background: Vec<u8> = serde_json::from_value(result["background_rgb"].clone()).unwrap();
        assert!(pixels.chunks(3).all(|pixel| pixel == background), "{mode}");
        assert_eq!(result["depth_range"], json!([0.0, 0.0]));
        assert_eq!(result["visibility"]["visible_object_count"], 0);
    }
    let (result, _) = capture(
        &mut document,
        json!({"projection":"perspective","position":[0,0,5],"yaw":0,"pitch":0,"hide_objects":[2]}),
    );
    assert_eq!(result["depth_range"], json!([-6.0, -4.0]));
    let (result, pixels) = capture(
        &mut document,
        json!({"solo_objects":[1,2],"hide_objects":[1,2]}),
    );
    assert_eq!(result["visibility"]["visible_object_count"], 0);
    assert!(pixels.chunks(3).all(|pixel| pixel == [232, 232, 232]));
}

#[test]
fn invalid_capture_filters_are_atomic_and_do_not_consume_views() {
    let mut document = Document::default();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    let (first, _) = capture(&mut document, json!({}));
    for filters in [
        json!({"hide_objects":[999]}),
        json!({"solo_objects":[-1]}),
        json!({"hide_objects":1}),
        json!({"solo_objects":null}),
        json!({"solo_objects":["1"]}),
    ] {
        let mut request = filters;
        request["command"] = json!("capture");
        assert!(document.execute(request).is_err());
        assert_eq!(
            document.execute(json!({"command":"inspect"})).unwrap(),
            before
        );
    }
    let (last, _) = capture(&mut document, json!({}));
    assert_eq!(
        last["view_id"].as_u64().unwrap(),
        first["view_id"].as_u64().unwrap() + 1
    );
}

#[test]
fn world_normals_preserve_winding_and_orientation_marks_backfaces() {
    let mut document = Document::default();
    let (_, pixels) = capture(&mut document, json!({"mode":"normals","yaw":0,"pitch":0}));
    assert_eq!(pixel(&pixels, 400, 300), [128, 128, 255]);
    let (_, pixels) = capture(&mut document, json!({"mode":"normals","yaw":180,"pitch":0}));
    assert_eq!(pixel(&pixels, 400, 300), [128, 128, 0]);
    let (_, pixels) = capture(
        &mut document,
        json!({"mode":"orientation","yaw":0,"pitch":0}),
    );
    assert_eq!(pixel(&pixels, 400, 300), [180; 3]);
    let mut project = document.execute(json!({"command":"save"})).unwrap();
    project["mesh"]["faces"]["2"] = json!([8, 7, 6, 5]);
    document
        .execute(json!({"command":"load","project":project}))
        .unwrap();
    let (_, pixels) = capture(&mut document, json!({"mode":"normals","yaw":0,"pitch":0}));
    assert_eq!(pixel(&pixels, 400, 300), [128, 128, 0]);
    let (_, pixels) = capture(
        &mut document,
        json!({"mode":"orientation","yaw":0,"pitch":0}),
    );
    assert_eq!(pixel(&pixels, 400, 300), [220, 55, 40]);
}

fn crossing_faces() -> Document {
    let mut document = Document::default();
    document.execute(json!({"command":"load","project":{"version":1,"mesh":{
        "vertices":{"1":[-1,-1,0],"2":[1,-1,2],"3":[1,1,2],"4":[-1,1,0],"5":[-1,-1,1],"6":[1,-1,1],"7":[1,1,1],"8":[-1,1,1]},
        "faces":{"1":[1,2,3,4],"2":[5,6,7,8]},"selection":[]
    }}})).unwrap();
    document
}

#[test]
fn per_pixel_depth_resolves_crossing_faces_and_face_ids_have_an_exact_palette() {
    let mut document = crossing_faces();
    let (metadata, pixels) = capture(
        &mut document,
        json!({"mode":"face_ids","yaw":0,"pitch":0,"zoom":100}),
    );
    let identify = |x, y| {
        let rgb = pixel(&pixels, x, y);
        let key = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
        metadata["face_id_colors"][key].as_u64().unwrap()
    };
    assert_eq!(identify(350, 300), 2);
    assert_eq!(identify(450, 300), 1);
    assert_ne!(pixel(&pixels, 350, 300), pixel(&pixels, 450, 300));
    let (_, normals) = capture(&mut document, json!({"mode":"normals","yaw":0,"pitch":0}));
    assert_eq!(pixel(&normals, 350, 300), [128, 128, 255]);
    assert_eq!(pixel(&normals, 450, 300), [37, 128, 218]);
}

#[test]
fn depth_interpolates_across_faces_and_can_use_a_fixed_comparison_range() {
    let mut document = crossing_faces();
    let (metadata, pixels) = capture(&mut document, json!({"mode":"depth","yaw":0,"pitch":0}));
    assert_eq!(metadata["depth_range"], json!([0.0, 2.0]));
    assert_eq!(pixel(&pixels, 350, 300), [144; 3]);
    assert!(pixel(&pixels, 450, 300)[0] > pixel(&pixels, 350, 300)[0]);
    assert_eq!(pixel(&pixels, 0, 0), [0; 3]);
    let (_, fixed) = capture(
        &mut document,
        json!({"mode":"depth","yaw":0,"pitch":0,"depth_range":[0,4]}),
    );
    assert_eq!(pixel(&fixed, 350, 300), [88; 3]);
}

#[test]
fn topology_has_no_triangulation_diagonals_and_masks_are_binary() {
    let mut document = Document::default();
    let (_, topology) = capture(
        &mut document,
        json!({"mode":"topology","clean":true,"yaw":0,"pitch":0}),
    );
    let (_, shaded) = capture(&mut document, json!({"mode":"shaded","yaw":0,"pitch":0}));
    assert_eq!(pixel(&topology, 400, 300), pixel(&shaded, 400, 300));
    let (triangles, tessellation) = capture(
        &mut document,
        json!({"mode":"tessellation","yaw":0,"pitch":0}),
    );
    assert_eq!(pixel(&tessellation, 400, 300), [45; 3]);
    assert_eq!(triangles["tessellation"]["triangle_count"], 12);
    assert_eq!(triangles["tessellation"]["nonplanar_face_count"], 0);
    assert_eq!(pixel(&topology, 300, 350), [45; 3]);
    let (_, mask) = capture(
        &mut document,
        json!({"mode":"silhouette","yaw":0,"pitch":0}),
    );
    assert!(
        mask.chunks(3)
            .all(|pixel| pixel == [0; 3] || pixel == [255; 3])
    );
    assert_eq!(pixel(&mask, 400, 300), [0; 3]);
    assert_eq!(pixel(&mask, 0, 0), [255; 3]);
}

#[test]
fn modes_share_the_same_selection_projection_and_bad_requests_do_not_consume_views() {
    let mut document = Document::default();
    let (view, _) = capture(&mut document, json!({"mode":"face_ids","yaw":0,"pitch":0}));
    for request in [
        json!({"mode":"unknown"}),
        json!({"mode":null}),
        json!({"mode":"depth","depth_range":[2,1]}),
        json!({"mode":"depth","clean":false}),
        json!({"width":0}),
        json!({"height":63}),
        json!({"width":4097}),
        json!({"width":null}),
        json!({"height":300.5}),
        json!({"width":"400"}),
        json!({"width":4096,"height":2048}),
    ] {
        let mut request = request;
        request["command"] = json!("capture");
        assert!(document.execute(request).is_err());
    }
    let (next, _) = capture(&mut document, json!({"mode":"depth","yaw":0,"pitch":0}));
    assert_eq!(
        next["view_id"].as_u64().unwrap(),
        view["view_id"].as_u64().unwrap() + 1
    );
    let selected = document
        .execute(json!({"command":"select_image","view_id":view["view_id"],"rect":[0,0,800,600]}))
        .unwrap();
    assert_eq!(selected["mesh"]["selection"], json!([5, 6, 7, 8]));
}

#[test]
fn resolution_preserves_framing_and_pixel_selection_in_wide_portrait_and_large_captures() {
    for (width, height) in [(400, 300), (1600, 1200), (641, 321), (257, 513)] {
        let mut document = Document::default();
        let (view, pixels) = capture(
            &mut document,
            json!({"width":width,"height":height,"yaw":0,"pitch":0,"mode":"silhouette"}),
        );
        let scale = (width as f64 / 800.0).min(height as f64 / 600.0);
        assert_eq!(view["camera"]["pixels_per_unit"], 100.0 * scale);
        assert_eq!(pixels.len(), width * height * 3);
        let svg = view["svg"].as_str().unwrap();
        assert!(svg.contains(&format!("viewBox=\"0 0 {width} {height}\"")));
        let occupied: Vec<_> = pixels
            .chunks_exact(3)
            .enumerate()
            .filter(|(_, pixel)| *pixel == [0, 0, 0])
            .map(|(index, _)| (index % width, index / width))
            .collect();
        let left = occupied.iter().map(|p| p.0).min().unwrap();
        let right = occupied.iter().map(|p| p.0).max().unwrap();
        assert!((left as f64 - (width as f64 / 2.0 - 100.0 * scale)).abs() <= 1.0);
        assert!((right as f64 - (width as f64 / 2.0 + 100.0 * scale)).abs() <= 1.0);
        let x = width as f64 / 2.0 - 100.0 * scale;
        let y = height as f64 / 2.0 + 100.0 * scale;
        let selected = document.execute(json!({"command":"select_image","view_id":view["view_id"],"rect":[x-0.1,y-0.1,x+0.1,y+0.1]})).unwrap();
        assert_eq!(selected["mesh"]["selection"], json!([5]));
    }
}

#[test]
fn every_render_mode_and_annotations_use_the_requested_canvas() {
    let mut document = Document::default();
    for mode in [
        "shaded",
        "topology",
        "normals",
        "depth",
        "silhouette",
        "face_ids",
        "orientation",
    ] {
        let (metadata, pixels) =
            capture(&mut document, json!({"width":320,"height":240,"mode":mode}));
        assert_eq!(metadata["width"], 320);
        assert_eq!(metadata["height"], 240);
        assert_eq!(pixels.len(), 320 * 240 * 3);
    }
    let (_, pixels) = capture(
        &mut document,
        json!({"width":320,"height":240,"mode":"topology","clean":false}),
    );
    assert!(pixels.chunks_exact(3).any(|pixel| pixel == [35, 35, 35]));
}

#[test]
fn smooth_normals_affect_captures_while_crease_angles_preserve_hard_edges() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"shade_smooth","object":1,"angle":45}))
        .unwrap();
    let (_, hard) = capture(&mut document, json!({"mode":"normals","yaw":0,"pitch":0}));
    assert_eq!(pixel(&hard, 310, 210), [128, 128, 255]);
    document
        .execute(json!({"command":"shade_smooth","object":1,"angle":180}))
        .unwrap();
    let (_, smooth) = capture(&mut document, json!({"mode":"normals","yaw":0,"pitch":0}));
    assert!(pixel(&smooth, 310, 210)[0] < 100);
    assert!(pixel(&smooth, 310, 210)[1] > 150);
    let before = document.execute(json!({"command":"save"})).unwrap();
    document
        .execute(json!({"command":"capture","mode":"orientation"}))
        .unwrap();
    assert_eq!(before, document.execute(json!({"command":"save"})).unwrap());
}
