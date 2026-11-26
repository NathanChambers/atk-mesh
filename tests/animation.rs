use atk_mesh::Document;
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
fn joint_limits_constrain_sampled_rotation_without_rewriting_keys() {
    let mut document = fixture();
    document.execute(json!({"command":"rig_rotation_limits_set","limits":{
        "upper":{"axis":[2,0,0],"max_swing":30,"twist":[-10,20]}
    }})).unwrap();
    document.execute(json!({"command":"animation_track_set","name":"Move","bone":"upper",
        "keys":[{"time":0,"rotation":[90,0,90]}]})).unwrap();

    let posed = document.execute(json!({"command":"animation_sample","clip":"Move","time":0,
        "bones":["upper","tip"]})).unwrap();
    let tip = posed["items"][1]["position"].as_array().unwrap();
    assert!((tip[0].as_f64().unwrap() - 3.0_f64.sqrt()).abs() < 1e-12);
    assert!((tip[1].as_f64().unwrap() - 1.0).abs() < 1e-12);
    let rotation = posed["items"][0]["local_rotation"].as_array().unwrap();
    let twist = 2.0 * rotation[0].as_f64().unwrap().atan2(rotation[3].as_f64().unwrap());
    assert!((twist.to_degrees() - 20.0).abs() < 1e-12);
    let keys = document.execute(json!({"command":"query_animation","clip":"Move","bone":"upper"})).unwrap();
    assert_eq!(keys["items"][0]["rotation"], json!([90.0,0.0,90.0]));

    assert!(document.execute(json!({"command":"rig_rotation_limits_set","limits":{
        "upper":{"axis":[0,0,0],"max_swing":30,"twist":[-10,20]}
    }})).is_err());
    assert_eq!(sample(&mut document, 0.0)["items"][0]["position"], posed["items"][1]["position"]);
    document.execute(json!({"command":"rig_rotation_limits_set","limits":{"upper":null}})).unwrap();
    assert!((sample(&mut document, 0.0)["items"][0]["position"][1].as_f64().unwrap() - 2.0).abs() < 1e-12);
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
    let bytes = atk_mesh::base64::decode(result["glb_base64"].as_str().unwrap()).unwrap();
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
#[test]
fn projected_weights_and_shared_curve_match_explicit_rig_data() {
    let mut document = atk_mesh::Document::default();
    document.execute(serde_json::json!({
        "command":"rig_set","rig":{"bones":[
            {"name":"parent","position":[0,0,0]},
            {"name":"child","parent":"parent","position":[0,1,0]}
        ]}
    })).unwrap();
    let run = |document: &mut atk_mesh::Document, request| document.execute(request).unwrap();
    run(&mut document, serde_json::json!({
        "command":"skin_weights_set","vertices":[1,2,3],
        "ramp":{"bones":["parent","child"],"start":[-2,0,0],"end":[2,0,0],"easing":"smoothstep"}
    }));
    run(&mut document, serde_json::json!({
        "command":"animation_set","name":"Shared","clip":{"duration":1},
        "shared_curve":{"keys":[[0,0],[0.5,-0.5],[1,1]],"channels":{
            "parent":{"rotation":[0,0,20]},"child":{"translation":[0,0,0.5]}
        }}
    }));
    let query = serde_json::json!({"command":"query_rig","response":"full"});
    let generated = run(&mut document, query.clone());
    run(&mut document, serde_json::json!({
        "command":"skin_weights_set","weights":{
            "1":[{"bone":"parent","weight":0.84375},{"bone":"child","weight":0.15625}],
            "2":[{"bone":"parent","weight":0.15625},{"bone":"child","weight":0.84375}],
            "3":[{"bone":"parent","weight":0.15625},{"bone":"child","weight":0.84375}]
        }
    }));
    run(&mut document, serde_json::json!({
        "command":"animation_set","name":"Shared","clip":{"duration":1,"tracks":{
            "parent":[{"time":0},{"time":0.5,"rotation":[0,0,-10]},{"time":1,"rotation":[0,0,20]}],
            "child":[{"time":0},{"time":0.5,"translation":[0,0,-0.25]},{"time":1,"translation":[0,0,0.5]}]
        }}
    }));
    assert_eq!(generated, run(&mut document, query));
}

#[test]
fn rigid_binding_matches_explicit_weights_for_one_object() {
    let mut document = fixture();
    let created = document.execute(json!({
        "command":"object_instance","object":1,"name":"Collar","value":[3,0,0]
    })).unwrap();
    let object = created["operation"]["object"].as_u64().unwrap();
    let query = json!({"command":"query_rig","response":"full"});
    let before = document.execute(query.clone()).unwrap();
    document.execute(json!({"command":"skin_weights_set","object":object,"bone":"upper"})).unwrap();
    let rigid = document.execute(query.clone()).unwrap();
    assert_eq!(rigid["rig"]["weights"]["1"], before["rig"]["weights"]["1"]);
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(before, document.execute(query.clone()).unwrap());
    let weights: std::collections::BTreeMap<_, _> = (9..=16)
        .map(|id| (id, json!([{"bone":"upper","weight":1.0}])))
        .collect();
    document.execute(json!({"command":"skin_weights_set","weights":weights})).unwrap();
    assert_eq!(rigid, document.execute(query.clone()).unwrap());
    assert!(document.execute(json!({"command":"skin_weights_set","object":object,"bone":"missing"})).is_err());
    assert_eq!(rigid, document.execute(query).unwrap());
}
