use atk_mesh::{Document, session::Session};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("mesh-glb-{}-{sequence}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, mut source: Value) -> PathBuf {
        let mut binary = vec![];
        for position in [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for component in position {
                binary.extend(component.to_le_bytes());
            }
            binary.extend([0; 4]);
        }
        binary.extend([0, 1, 2, 0]);
        source["asset"] = json!({"version":"2.0"});
        source["buffers"] = json!([{"byteLength":binary.len()}]);
        source["bufferViews"] = json!([{"buffer":0,"byteLength":48,"byteStride":16},{"buffer":0,"byteOffset":48,"byteLength":3}]);
        if source.get("accessors").is_none() {
            source["accessors"] = json!([{"bufferView":0,"componentType":5126,"type":"VEC3","count":3},{"bufferView":1,"componentType":5121,"type":"SCALAR","count":3}]);
        }
        if source.get("meshes").is_none() {
            source["meshes"] = json!([{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}]);
        }
        let mut encoded = serde_json::to_vec(&source).unwrap();
        while encoded.len() % 4 != 0 {
            encoded.push(b' ');
        }
        let length = 12 + 8 + encoded.len() + 8 + binary.len();
        let mut bytes = b"glTF".to_vec();
        bytes.extend(2_u32.to_le_bytes());
        bytes.extend((length as u32).to_le_bytes());
        bytes.extend((encoded.len() as u32).to_le_bytes());
        bytes.extend(0x4e4f534a_u32.to_le_bytes());
        bytes.extend(encoded);
        bytes.extend((binary.len() as u32).to_le_bytes());
        bytes.extend(0x004e4942_u32.to_le_bytes());
        bytes.extend(binary);
        let path = self.0.join("fixture.glb");
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn source() -> Value {
    json!({"scenes":[{"nodes":[0]}],"nodes":[{"translation":[2,3,4],"children":[1,2]},{"name":"Part","mesh":0,"scale":[-2,1,1]},{"name":"Part","mesh":0,"translation":[0,0,5]}]})
}

#[test]
fn import_appends_transformed_objects_and_is_undoable() {
    let fixture = Fixture::new();
    let path = fixture.write(source());
    let mut document = Document::default();
    let before = document.execute(json!({"command":"save"})).unwrap();
    let imported = document
        .execute(json!({"command":"import_glb","path":path}))
        .unwrap();
    assert_eq!(imported["operation"]["objects"], json!([2, 3]));
    assert_eq!(imported["mesh"]["objects"]["2"]["name"], "Part");
    assert_eq!(imported["mesh"]["objects"]["3"]["name"], "Part 2");
    assert_eq!(
        imported["mesh"]["objects"]["2"]["origin"],
        json!([2.0, 3.0, 4.0])
    );
    assert_eq!(imported["mesh"]["vertices"]["10"], json!([0.0, 3.0, 4.0]));
    assert_eq!(imported["mesh"]["vertices"]["12"], json!([2.0, 3.0, 9.0]));
    assert_eq!(imported["mesh"]["faces"]["7"], json!([9, 11, 10]));
    assert_eq!(imported["mesh"]["faces"]["8"], json!([12, 13, 14]));
    assert_eq!(
        imported["mesh"]["objects"]["1"],
        before["mesh"]["objects"]["1"]
    );
    assert_eq!(
        document.execute(json!({"command":"undo"})).unwrap()["mesh"],
        before["mesh"]
    );
    assert_eq!(
        document.execute(json!({"command":"redo"})).unwrap()["mesh"],
        imported["mesh"]
    );
    let saved = document.execute(json!({"command":"save"})).unwrap();
    let mut restored = Document::default();
    assert_eq!(
        restored
            .execute(json!({"command":"load","project":saved}))
            .unwrap()["mesh"],
        imported["mesh"]
    );
}

#[test]
fn invalid_imports_roll_back_geometry_ids_and_history() {
    let fixture = Fixture::new();
    let mut document = Document::default();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    let mut invalid = source();
    invalid["nodes"][2]["mesh"] = json!(99);
    let path = fixture.write(invalid);
    assert!(
        document
            .execute(json!({"command":"import_glb","path":path}))
            .is_err()
    );
    let after = document.execute(json!({"command":"inspect"})).unwrap();
    assert_eq!(after, before);

    let mut cyclic = source();
    cyclic["nodes"][2]["children"] = json!([0]);
    let path = fixture.write(cyclic);
    assert!(
        document
            .execute(json!({"command":"import_glb","path":path}))
            .is_err()
    );
    let path = fixture.write(source());
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.pop();
    std::fs::write(&path, bytes).unwrap();
    assert!(
        document
            .execute(json!({"command":"import_glb","path":path}))
            .is_err()
    );
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
}

#[test]
fn session_resolves_workspace_paths_records_and_restores_imports() {
    let fixture = Fixture::new();
    fixture.write(source());
    let mut session = Session::open_workspace(fixture.0.clone()).unwrap();
    let imported = session.execute(json!({"command":"import_glb","path":"fixture.glb"}));
    assert_eq!(imported["ok"], true, "{imported}");
    assert!(imported["trace"]["session"].is_string());
    let state = session.state();
    drop(session);
    let mut session = Session::open_workspace(fixture.0.clone()).unwrap();
    assert_eq!(session.state()["mesh"], state["mesh"]);
    let outside = Fixture::new();
    let path = outside.write(source());
    assert_eq!(
        session.execute(json!({"command":"import_glb","path":path}))["ok"],
        false
    );
    assert_eq!(session.state()["mesh"], state["mesh"]);
}

#[test]
fn nonindexed_triangles_bake_column_major_matrix_and_quaternion() {
    let fixture = Fixture::new();
    let path = fixture.write(json!({
        "scenes":[{"nodes":[0]}],
        "nodes":[
            {"matrix":[1,0,0,0,0,1,0,0,0,0,1,0,2,3,4,1],"children":[1]},
            {"mesh":0,"rotation":[0,0,1,0]}
        ],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]
    }));
    let mut document = Document::default();
    let imported = document
        .execute(json!({"command":"import_glb","path":path}))
        .unwrap();
    assert_eq!(imported["mesh"]["vertices"]["9"], json!([2.0, 3.0, 4.0]));
    assert_eq!(imported["mesh"]["vertices"]["10"], json!([1.0, 3.0, 4.0]));
    assert_eq!(imported["mesh"]["vertices"]["11"], json!([2.0, 2.0, 4.0]));
}

#[test]
fn unsupported_features_and_out_of_bounds_accessors_fail_atomically() {
    let fixture = Fixture::new();
    let mut document = Document::default();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    for (pointer, value) in [
        ("/extensionsRequired", json!(["KHR_draco_mesh_compression"])),
        ("/animations", json!([{}])),
        ("/nodes/1/skin", json!(0)),
        ("/nodes/1/scale", json!([0, 1, 1])),
        ("/nodes/1/rotation", json!([0, 0, 0, 0])),
    ] {
        let mut invalid = source();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        invalid.pointer_mut(parent).unwrap()[key] = value;
        let path = fixture.write(invalid);
        assert!(
            document
                .execute(json!({"command":"import_glb","path":path}))
                .is_err(),
            "{pointer}"
        );
        assert_eq!(
            document.execute(json!({"command":"inspect"})).unwrap(),
            before
        );
    }
    let mut invalid = source();
    invalid["accessors"] = json!([{"bufferView":0,"componentType":5126,"type":"VEC3","count":4}]);
    let path = fixture.write(invalid);
    assert!(
        document
            .execute(json!({"command":"import_glb","path":path}))
            .is_err()
    );
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
}
