use gpt_mesh::Document;
use serde_json::{Value, json};

fn fixture() -> Document {
    let mut document = Document::default();
    document.execute(json!({"command":"rig_set","rig":{"bones":[
        {"name":"body","position":[0,0,0]},
        {"name":"upper","parent":"body","position":[0,0,0]},
        {"name":"lower","parent":"upper","position":[1,0,0]},
        {"name":"tip","parent":"lower","position":[2,0,0]}
    ],"weights":{"1":[{"bone":"tip","weight":1}]}}})).unwrap();
    document.execute(json!({"command":"animation_set","name":"Move","clip":{
        "duration":2,"tracks":{"body":[{"time":0},{"time":2,"translation":[0,0,0.4]}]}
    }})).unwrap();
    document
}

fn sample(document: &mut Document, time: f64) -> Value {
    document.execute(json!({"command":"animation_sample","clip":"Move","time":time,
        "bones":["tip"]})).unwrap()
}

#[test]
fn display_tails_preserve_animation_and_reject_invalid_edits_atomically() {
    let mut document = fixture();
    let before = sample(&mut document, 0.5);
    document.execute(json!({"command":"rig_display_tails_set","tails":{"tip":[2,1,0]}})).unwrap();
    let rig = document.execute(json!({"command":"query_rig","response":"full"})).unwrap();
    assert_eq!(rig["rig"]["bones"][3]["tail"], json!([2.0,1.0,0.0]));
    assert_eq!(sample(&mut document, 0.5)["items"], before["items"]);
    assert!(document.execute(json!({"command":"rig_display_tails_set","tails":{"body":[0,1,0],"tip":[2,0,0]}})).is_err());
    assert_eq!(document.execute(json!({"command":"query_rig","response":"full"})).unwrap(), rig);
    document.execute(json!({"command":"rig_display_tails_set","tails":{"tip":null}})).unwrap();
    let rig = document.execute(json!({"command":"query_rig","response":"full"})).unwrap();
    assert!(rig["rig"]["bones"][3].get("tail").is_none());
}

#[test]
fn sampling_and_local_edits_support_step_cubic_retime_and_atomic_failure() {
    let mut document = fixture();
    assert_eq!(sample(&mut document, 0.5)["items"][0]["position"], json!([2.0,0.0,0.1]));
    document.execute(json!({"command":"animation_track_set","name":"Move","bone":"body","interpolation":"step"})).unwrap();
    assert_eq!(sample(&mut document, 0.5)["items"][0]["position"], json!([2.0,0.0,0.0]));
    document.execute(json!({"command":"animation_track_set","name":"Move","bone":"body","interpolation":"cubic"})).unwrap();
    let quarter = sample(&mut document, 0.5);
    assert!((quarter["items"][0]["position"][2].as_f64().unwrap() - 0.0625).abs() < 1e-12);
    assert!(quarter["bounds"]["min"].is_array());
    let poses = document.execute(json!({"command":"animation_sample","kind":"vertices","clip":"Move","time":0.5,"vertices":[1,2],"limit":1})).unwrap();
    assert_eq!(poses["total"], 2);
    assert_eq!(poses["next_offset"], 1);
    assert!((poses["items"][0]["position"][2].as_f64().unwrap() + 0.9375).abs() < 1e-12);

    document.execute(json!({"command":"animation_keys_edit","name":"Move","edits":[
        {"op":"set","bone":"body","key":{"time":1,"translation":[0,0,0.5]}},
        {"op":"retime","bones":["body"],"start":0,"end":1,"shift":0.25},
        {"op":"remove","bone":"body","time":2}
    ]})).unwrap();
    let keys = document.execute(json!({"command":"query_animation","clip":"Move","bone":"body"})).unwrap();
    assert_eq!(keys["items"][0]["time"], 0.25);
    assert_eq!(keys["items"][1]["time"], 1.25);
    let revision = document.revision();
    assert!(document.execute(json!({"command":"animation_keys_edit","name":"Move","edits":[
        {"op":"remove","bone":"body","time":0.25},
        {"op":"retime","bones":["body"],"start":1,"end":2,"shift":1}
    ]})).is_err());
    assert_eq!(document.revision(), revision);
    assert_eq!(document.execute(json!({"command":"query_animation","clip":"Move","bone":"body"})).unwrap(), keys);
}

#[test]
fn two_bone_bake_holds_a_world_target_through_parent_motion() {
    let mut document = fixture();
    let result = document.execute(json!({"command":"animation_ik_bake","name":"Move",
        "chain":["upper","lower","tip"],"target":[1.35,0,0],"pole":[0,1,0],
        "start":0.5,"end":1.5,"fps":30,"tip_rotation":[0,0,0]})).unwrap();
    assert_eq!(result["operation"]["samples"], 31);
    assert_eq!(result["operation"]["clamped_samples"], 0);
    for time in [0.5, 0.8, 1.0, 1.23, 1.5] {
        let pose = sample(&mut document, time);
        let tip = &pose["items"][0];
        for (axis, expected) in [1.35, 0.0, 0.0].into_iter().enumerate() {
            assert!((tip["position"][axis].as_f64().unwrap() - expected).abs() < 1e-4, "{tip}");
        }
        assert!(tip["rotation"][3].as_f64().unwrap().abs() > 0.999999);
    }
    assert!(sample(&mut document, 0.0)["items"][0]["position"][0].as_f64().unwrap() > 1.99);
    assert!(sample(&mut document, 2.0)["items"][0]["position"][0].as_f64().unwrap() > 1.99);
}

#[test]
fn animated_glb_encodes_skin_bindings_root_motion_and_clip_interpolation() {
    let mut document = fixture();
    document.execute(json!({"command":"animation_track_set","name":"Move","bone":"body","interpolation":"cubic"})).unwrap();
    document.execute(json!({"command":"animation_track_set","name":"Move","bone":"tip","interpolation":"step",
        "keys":[{"time":0},{"time":1,"rotation":[0,0,45]}]})).unwrap();
    let result = document.execute(json!({"command":"export","format":"glb"})).unwrap();
    let bytes = gpt_mesh::base64::decode(result["glb_base64"].as_str().unwrap()).unwrap();
    assert_eq!(&bytes[..4], b"glTF");
    assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize, bytes.len());
    let json_size = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let data: Value = serde_json::from_slice(&bytes[20..20+json_size]).unwrap();
    let binary = &bytes[28+json_size..];
    let accessor = |id: &Value| {
        let record = &data["accessors"][id.as_u64().unwrap() as usize];
        let view = &data["bufferViews"][record["bufferView"].as_u64().unwrap() as usize];
        let start = view["byteOffset"].as_u64().unwrap() as usize;
        let size = view["byteLength"].as_u64().unwrap() as usize;
        &binary[start..start+size]
    };
    let skin = &data["skins"][0];
    assert_eq!(skin["joints"], json!([1,2,3,4,5]));
    let inverse = accessor(&skin["inverseBindMatrices"]);
    assert_eq!(f32::from_le_bytes(inverse[3*64+48..3*64+52].try_into().unwrap()), -2.0);
    let attributes = &data["meshes"][0]["primitives"][0]["attributes"];
    assert!(accessor(&attributes["JOINTS_0"]).chunks_exact(8).any(|vertex| vertex[..2] == 4u16.to_le_bytes()));
    let animation = &data["animations"][0];
    assert_eq!(animation["name"], "Move");
    assert_eq!(animation["samplers"][2]["interpolation"], "STEP");
    let translations = accessor(&animation["samplers"][1]["output"]);
    let times = accessor(&animation["samplers"][1]["input"]);
    assert_eq!(times.len() / 4, 121);
    assert_eq!(f32::from_le_bytes(translations[30*12+8..30*12+12].try_into().unwrap()), 0.0625);
}
