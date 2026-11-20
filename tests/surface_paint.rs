use gpt_mesh::{Document, base64};
use serde_json::{Value, json};

fn fixture(project: &str) -> Document {
    let mut document = Document::default();
    let project: Value = serde_json::from_str(project).unwrap();
    run(&mut document, json!({"command":"load","project":project}));
    run(
        &mut document,
        json!({"command":"texture_create","texture":"atlas","width":128,"height":128,"color":[0,0,0,1]}),
    );
    run(
        &mut document,
        json!({"command":"texture_assign","texture":"atlas"}),
    );
    document
}

fn run(document: &mut Document, request: Value) -> Value {
    document
        .execute(request.clone())
        .unwrap_or_else(|error| panic!("{request}: {error}"))
}

fn pixels(document: &mut Document) -> Vec<u8> {
    let saved = run(document, json!({"command":"save"}));
    base64::decode(
        saved["mesh"]["textures"]["atlas"]["rgba_base64"]
            .as_str()
            .unwrap(),
    )
    .unwrap()
}

fn red(pixels: &[u8], x: usize, y_from_bottom: usize) -> u8 {
    pixels[((127 - y_from_bottom) * 128 + x) * 4]
}

fn add_shared_fill(document: &mut Document) {
    run(
        document,
        json!({"command":"texture_layer_add","texture":"atlas","layer":{"name":"white","mask":{"source":"shape"}}}),
    );
}

#[test]
fn cube_face_expansion_and_named_region_mask() {
    let mut cube = fixture(include_str!("fixtures/texturing/atlas-cube/project.json"));
    let island = run(
        &mut cube,
        json!({"command":"query_texture_faces","seed_face":1,"expand":"uv_island"}),
    );
    assert_eq!(island["faces"], json!([1]));
    let connected = run(
        &mut cube,
        json!({"command":"query_texture_faces","seed_face":1,"expand":"connected"}),
    );
    assert_eq!(connected["faces"], json!([1, 2, 3, 4, 5, 6]));
    run(
        &mut cube,
        json!({"command":"region_define","name":"lower-left","faces":[1]}),
    );
    run(
        &mut cube,
        json!({"command":"texture_layer_add","texture":"atlas","layer":{"name":"white"}}),
    );
    run(
        &mut cube,
        json!({"command":"texture_layer_mask_faces","texture":"atlas","layer":"white","region":"lower-left"}),
    );
    let region = pixels(&mut cube);
    assert_eq!(red(&region, 24, 40), 255);
    assert_eq!(red(&region, 64, 40), 0);
    run(
        &mut cube,
        json!({"command":"texture_layer_mask_faces","texture":"atlas","layer":"white","seed_face":1,"expand":"uv_island"}),
    );
    assert_eq!(pixels(&mut cube), region);
}

#[test]
fn cylinder_band_crosses_seam_and_survives_edit_save_and_undo() {
    let mut cylinder = fixture(include_str!(
        "fixtures/texturing/seam-cylinder/project.json"
    ));
    let island = run(
        &mut cylinder,
        json!({"command":"query_texture_faces","seed_face":1,"expand":"uv_island"}),
    );
    assert_eq!(island["faces"], json!((1..=16).collect::<Vec<_>>()));
    run(
        &mut cylinder,
        json!({"command":"texture_mask_surface","texture":"atlas","name":"shape","seed_face":1,"expand":"uv_island","shape":{"kind":"band","center":[0,0,0],"axis":[0,1,0],"width":0.25,"feather":0.05}}),
    );
    add_shared_fill(&mut cylinder);
    let narrow = pixels(&mut cylinder);
    for x in 8..120 {
        assert_eq!(red(&narrow, x, 32), 255);
        assert_eq!(red(&narrow, x, 40), 0);
    }
    run(
        &mut cylinder,
        json!({"command":"texture_mask_surface","texture":"atlas","name":"shape","shape":{"kind":"band","center":[0,0,0],"axis":[0,1,0],"width":1}}),
    );
    let wide = pixels(&mut cylinder);
    for x in 8..120 {
        assert_eq!(red(&wide, x, 40), 255);
    }
    assert_eq!(red(&wide, 32, 96), 0);
    assert_eq!(red(&wide, 96, 96), 0);
    let saved = run(&mut cylinder, json!({"command":"save"}));
    assert_eq!(saved["version"], 8);
    let mut reopened = Document::default();
    run(&mut reopened, json!({"command":"load","project":saved}));
    run(
        &mut reopened,
        json!({"command":"texture_bake","texture":"atlas"}),
    );
    assert_eq!(pixels(&mut reopened), wide);
    run(&mut cylinder, json!({"command":"undo"}));
    assert_eq!(pixels(&mut cylinder), narrow);
}

#[test]
fn decal_moves_without_reimport_and_projects_on_both_sides_of_uv_seam() {
    let mut panel = fixture(include_str!("fixtures/texturing/panel/project.json"));
    run(
        &mut panel,
        json!({"command":"texture_create","texture":"alpha","width":8,"height":8,"color":[1,0,0,1]}),
    );
    run(
        &mut panel,
        json!({"command":"texture_mask_surface","texture":"atlas","name":"shape","source_texture":"alpha","shape":{"kind":"decal","center":[-0.5,0,0],"right":[1,0,0],"up":[0,1,0],"size":[0.5,0.5],"depth":0.1}}),
    );
    add_shared_fill(&mut panel);
    let initial = pixels(&mut panel);
    assert_eq!(red(&initial, 32, 64), 255);
    assert_eq!(red(&initial, 96, 64), 0);
    run(
        &mut panel,
        json!({"command":"texture_remove","texture":"alpha"}),
    );
    run(
        &mut panel,
        json!({"command":"texture_mask_surface","texture":"atlas","name":"shape","shape":{"kind":"decal","center":[0.5,0,0],"right":[0,1,0],"up":[-1,0,0],"size":[1,0.25],"depth":0.1}}),
    );
    let moved = pixels(&mut panel);
    assert_eq!(red(&moved, 32, 64), 0);
    assert_eq!(red(&moved, 96, 90), 255);
    assert_eq!(red(&moved, 107, 64), 0);
    let before = run(&mut panel, json!({"command":"save"}));
    assert!(panel.execute(json!({"command":"texture_mask_surface","texture":"atlas","name":"shape","shape":{"kind":"decal","center":[0,0,0],"right":[1,0,0],"up":[1,0,0],"size":[1,1],"depth":1}})).is_err());
    assert_eq!(run(&mut panel, json!({"command":"save"})), before);

    let mut cylinder = fixture(include_str!(
        "fixtures/texturing/seam-cylinder/project.json"
    ));
    run(
        &mut cylinder,
        json!({"command":"texture_create","texture":"alpha","width":8,"height":8,"color":[1,0,0,1]}),
    );
    run(
        &mut cylinder,
        json!({"command":"texture_mask_surface","texture":"atlas","name":"shape","source_texture":"alpha","shape":{"kind":"decal","center":[1,0,0],"right":[0,0,-1],"up":[0,1,0],"size":[0.8,0.8],"depth":0.3}}),
    );
    add_shared_fill(&mut cylinder);
    let projected = pixels(&mut cylinder);
    assert_eq!(red(&projected, 8, 32), 255);
    assert_eq!(red(&projected, 119, 32), 255);
    assert_eq!(red(&projected, 64, 32), 0);
}

#[test]
fn gradient_tracks_cube_height_across_uv_islands() {
    let mut cube = fixture(include_str!("fixtures/texturing/atlas-cube/project.json"));
    run(
        &mut cube,
        json!({"command":"texture_mask_surface","texture":"atlas","name":"shape","shape":{"kind":"gradient","start":[0,-1,0],"end":[0,1,0]}}),
    );
    add_shared_fill(&mut cube);
    let gradient = pixels(&mut cube);
    assert_eq!(red(&gradient, 104, 40), 0);
    assert_eq!(red(&gradient, 24, 88), 255);
    let halfway = red(&gradient, 64, 40);
    assert!((120..=135).contains(&halfway), "{halfway}");
    assert_eq!(red(&gradient, 1, 1), 0);
}
