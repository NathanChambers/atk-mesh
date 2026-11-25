import assert from 'node:assert/strict';
import test from 'node:test';
import { setMaxListeners } from 'node:events';
import { buildMeshData, buildCornerNormals, cameraMatrix, measureMesh, orthographic, perspective, facePalette, triangulateFace } from '../web/geometry.js';
import { ModelViewer } from '../web/renderer.js';
import { createAnimationSampler } from '../web/animation.js';
import { SessionReplay } from '../web/replay.js';
import { ObjectInspector } from '../web/objects.js';

test('concave repaired faces triangulate their surface without filling a notch', () => {
    const points = [[0, 0, 0], [3, 0, 0], [3, 3, 0], [2, 3, 0], [2, 1, 0], [1, 1, 0], [1, 3, 0], [0, 3, 0]];
    const triangles = triangulateFace(points);
    const cross = (a, b, c) => (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let area = 0;
    for (const indices of triangles) {
        const [a, b, c] = indices.map(index => points[index]);
        area += cross(a, b, c) / 2;
        const notch = [1.5, 2];
        const inside = cross(a, b, notch) >= 0 && cross(b, c, notch) >= 0 && cross(c, a, notch) >= 0;
        assert.equal(inside, false);
    }
    assert.equal(area, 7);
    assert.deepEqual(triangulateFace(points.slice(0, 3)), [[0, 1, 2]]);
});

const mesh = {
    vertices: { 1: [-1, -1, 0], 2: [1, -1, 0], 3: [1, 1, 0], 4: [-1, 1, 0] },
    faces: { 1: [1, 2, 3, 4] }
};

test('smooth normals cross shallow edges and split at the configured crease angle', () => {
    const folded = { vertices: { ...mesh.vertices, 5: [1, 1, -2], 6: [-1, 1, -2] }, faces: { 1: [1, 2, 3, 4], 2: [4, 3, 5, 6] }, objects: { 1: { vertices: [1, 2, 3, 4, 5, 6], smooth_angle: 120 } } };
    let normals = buildCornerNormals(folded);
    const expected = [0, Math.SQRT1_2, Math.SQRT1_2];
    for (let axis = 0; axis < 3; axis++) { assert.ok(Math.abs(normals[0][2][axis] - expected[axis]) < 1e-10); }
    assert.deepEqual(normals[0][2], normals[1][1]);
    folded.objects[1].smooth_angle = 45;
    normals = buildCornerNormals(folded);
    assert.deepEqual(normals[0][2], [0, 0, 1]);
    assert.deepEqual(normals[1][1], [0, 1, 0]);
    folded.objects[1].smooth_angle = 120;
    folded.flat_faces = [1];
    normals = buildCornerNormals(folded);
    assert.deepEqual(normals[0][2], [0, 0, 1]);
    assert.deepEqual(normals[1][1], expected);
});

test('polygon projection builds flat normals and perimeter edges without triangulation diagonals', () => {
    const data = buildMeshData(mesh, [0, 0, 0], 1);
    assert.equal(data.positions.length, 18);
    assert.equal(data.edges.length, 24);
    for (let index = 0; index < data.normals.length; index += 3) {
        assert.deepEqual(Array.from(data.normals.slice(index, index + 3)), [0, 0, 1]);
    }
});

test('bounds and normalization support offset meshes and preserve their source coordinates', () => {
    const source = structuredClone(mesh);
    const bounds = measureMesh(mesh);
    assert.deepEqual(bounds.center, [0, 0, 0]);
    assert.equal(bounds.radius, Math.sqrt(2));
    const data = buildMeshData(mesh, [3, 2, 1], 2);
    assert.deepEqual(Array.from(data.positions.slice(0, 3)), [-2, -1.5, -0.5]);
    assert.deepEqual(mesh, source);
});

test('shared polygon edges are emitted once', () => {
    const adjacent = { ...mesh, faces: { 1: [1, 2, 3], 2: [1, 3, 4] } };
    const data = buildMeshData(adjacent, [0, 0, 0], 1);
    assert.equal(data.edges.length, 30);
});

test('face ID colors are vivid, unique and constant across each polygon', () => {
    const palette = facePalette(10000);
    const unique = new Set(palette.map(color => color.join(',')));
    assert.equal(unique.size, 10000);
    for (const color of palette.slice(0, 78)) {
        assert.ok(Math.max(...color) - Math.min(...color) > 150);
    }
    const data = buildMeshData(mesh, [0, 0, 0], 1);
    assert.equal(data.faceColors.length, data.positions.length);
    for (let index = 3; index < data.faceColors.length; index += 3) {
        assert.deepEqual(data.faceColors.slice(index, index + 3), data.faceColors.slice(0, 3));
    }
});

test('camera puts target in front and perspective maps it inside the clip volume', () => {
    const matrix = cameraMatrix([0, 0, 4], [0, 0, 0]);
    assert.equal(matrix[14], -4);
    const projection = perspective(Math.PI / 4, 16 / 9, 0.1, 100);
    const clipDepth = projection[10] * -4 + projection[14];
    assert.ok(clipDepth / 4 > -1 && clipDepth / 4 < 1);
    for (const value of [...matrix, ...projection]) { assert.ok(Number.isFinite(value)); }
});

test('orthographic axis views keep parallel lines parallel and have a stable basis at the poles', () => {
    const projection = orthographic(2, 2, -10, 10);
    assert.equal(projection[0], 0.25);
    assert.equal(projection[5], 0.5);
    assert.equal(projection[11], 0);
    assert.equal(projection[15], 1);
    for (const direction of [-1, 1]) {
        const matrix = cameraMatrix([2, 3 + 4 * direction, 5], [2, 3, 5]);
        assert.ok(matrix.every(Number.isFinite));
        assert.equal(matrix[0], 1);
        assert.equal(matrix[6], direction);
    }
});

function objectMesh() {
    return {
        vertices: { ...mesh.vertices, 5: [3, -1, 0], 6: [5, -1, 0], 7: [5, 1, 0], 8: [3, 1, 0] },
        faces: { 1: [1, 2, 3], 2: [1, 3, 4], 3: [5, 6, 7, 8] },
        objects: { 2: { name: 'Roof', origin: [0, 0, 0], vertices: [1, 2, 3, 4] }, 9: { name: 'Door', origin: [3, 0, 0], vertices: [5, 6, 7, 8] } }
    };
}

test('object colors group faces by ownership and preserve independent face colors', () => {
    const source = objectMesh();
    const data = buildMeshData(source, [0, 0, 0], 1);
    const palette = facePalette(2);
    const expected = palette.map(color => Array.from(new Float32Array(color.map(value => value / 255))));
    assert.deepEqual(Array.from(data.objectColors.slice(0, 3)), expected[0]);
    assert.deepEqual(Array.from(data.objectColors.slice(9, 12)), expected[0]);
    assert.deepEqual(Array.from(data.objectColors.slice(18, 21)), expected[1]);
    assert.notDeepEqual(data.faceColors.slice(0, 3), data.faceColors.slice(9, 12));
    assert.deepEqual(data.groups.map(group => [group.object, group.count, group.edgeCount]), [['2', 6, 10], ['9', 6, 8]]);
});

function browserEnvironment() {
    const calls = [];
    const gl = new Proxy({}, {
        get(target, name) {
            if (name === 'getShaderParameter' || name === 'getProgramParameter') { return () => true; }
            if (name === 'getAttribLocation') { return (_, attribute) => ({ position: 0, normal: 1, faceColor: 2, objectColor: 3, collectionColor: 4, uvCoordinate: 5, vertexColor: 6 })[attribute]; }
            if (name.startsWith('create')) { return () => ({}); }
            if (name === 'getUniformLocation') { return (_, uniform) => uniform; }
            if (name.toUpperCase() === name) { return name; }
            return (...arguments_) => { calls.push([name, ...arguments_]); };
        }
    });
    class Element extends EventTarget {
        constructor() {
            super();
            this.clientWidth = 1200;
            this.clientHeight = 800;
            this.width = 1200;
            this.height = 800;
            this.hidden = false;
            this.textContent = '';
            this.value = '';
            this.children = [];
            this.attributes = new Map();
        }
        getContext() { return gl; }
        setAttribute(name, value) { this.attributes.set(name, String(value)); }
        getAttribute(name) { return this.attributes.get(name) ?? null; }
        removeAttribute(name) { this.attributes.delete(name); }
        showModal() { this.open = true; }
        close() { this.open = false; }
        focus() {}
        scrollIntoView() {}
        replaceChildren(...children) { this.children = children; }
    }
    const elements = new Map();
    const document = new EventTarget();
    document.hidden = false;
    document.fullscreenEnabled = false;
    document.getElementById = id => {
        if (elements.has(id) === false) { elements.set(id, new Element()); }
        return elements.get(id);
    };
    document.querySelectorAll = () => [];
    document.createElement = () => new Element();
    const window = new EventTarget();
    window.devicePixelRatio = 1;
    window.location = new URL('http://localhost/');
    const replacements = {
        document, window,
        AbortController: class extends globalThis.AbortController {
            constructor() { super(); setMaxListeners(20, this.signal); }
        },
        ResizeObserver: class { observe() {} disconnect() {} },
        requestAnimationFrame: () => 1,
        cancelAnimationFrame: () => {}
    };
    const originals = new Map();
    function replace(name, value) {
        if (originals.has(name) === false) { originals.set(name, Object.getOwnPropertyDescriptor(globalThis, name)); }
        Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
    }
    for (const [name, value] of Object.entries(replacements)) { replace(name, value); }
    return {
        calls, document, window, replace,
        restore() {
            for (const [name, descriptor] of originals) {
                if (descriptor === undefined) { delete globalThis[name]; }
                else { Object.defineProperty(globalThis, name, descriptor); }
            }
        }
    };
}

test('camera gizmo keeps opposite axes selectable and switches projection independently', async () => {
    const environment = browserEnvironment();
    const element = id => environment.document.getElementById(id);
    environment.replace('fetch', async () => ({
        ok: true,
        json: async () => ({ ok: true, result: { mode: 'viewer', workspaces: [] } })
    }));
    const axes = ['right', 'left', 'top', 'bottom', 'front', 'back'].map(axis => element(`camera-${axis}`));
    try {
        await import('../web/app.js?camera-gizmo');
        element('orthographic').dispatchEvent(new Event('click'));
        assert.equal(element('orthographic').getAttribute('aria-pressed'), 'true');
        assert.equal(element('perspective').getAttribute('aria-pressed'), 'false');

        for (const axis of axes) {
            axis.dispatchEvent(new Event('click'));
            assert.deepEqual(axes.filter(button => button.getAttribute('aria-pressed') === 'true'), [axis]);
            const positions = axes.map(button =>
                [...button.getAttribute('style').matchAll(/\+ (-?[\d.e+-]+)px/g)].map(match => Number(match[1]))
            );
            positions.forEach(([x, y], index) => {
                assert.ok(Math.abs(x) <= 56 && Math.abs(y) <= 56, 'axis button fits the gizmo');
                for (const [otherX, otherY] of positions.slice(index + 1)) {
                    assert.ok(Math.hypot(x - otherX, y - otherY) >= 33, 'axis buttons leave a clear hit area');
                }
            });
        }

        assert.equal(element('orthographic').getAttribute('aria-pressed'), 'true');
        element('perspective').dispatchEvent(new Event('click'));
        assert.equal(element('perspective').getAttribute('aria-pressed'), 'true');
        assert.equal(element('orthographic').getAttribute('aria-pressed'), 'false');
        assert.equal(axes.at(-1).getAttribute('aria-pressed'), 'true');
    } finally {
        environment.window.dispatchEvent(new Event('pagehide'));
        environment.restore();
    }
});

test('visibility filters surface and wireframe draws without GPU uploads or mesh mutation', () => {
    const environment = browserEnvironment();
    let viewer;
    try {
        viewer = new ModelViewer(environment.document.getElementById('viewport'));
        const source = objectMesh();
        const before = structuredClone(source);
        viewer.setMesh(source);
        const uploads = environment.calls.filter(([name]) => name === 'bufferData').length;
        viewer.setObjectVisible(2, false);
        viewer.displayMode = 'object_ids';
        viewer.wireframe = true;
        environment.calls.length = 0;
        viewer.render();
        assert.deepEqual(environment.calls.filter(([name]) => name === 'drawArrays'), [['drawArrays', 'TRIANGLES', 6, 6], ['drawArrays', 'LINES', 10, 8]]);
        assert.equal(environment.calls.filter(([name]) => name === 'bufferData').length, 0);
        viewer.frame();
        assert.deepEqual(viewer.target, [2 / Math.sqrt(10), 0, 0]);
        viewer.setMesh(source);
        assert.equal(viewer.isObjectVisible(2), false);
        assert.ok(uploads > 0);
        viewer.isolateObjects([2]);
        assert.equal(viewer.isObjectVisible(9), false);
        viewer.showAllObjects();
        assert.equal(viewer.hiddenObjects.size, 0);
        viewer.isolateObjects([]);
        environment.calls.length = 0;
        viewer.render();
        assert.equal(environment.calls.filter(([name]) => name === 'drawArrays').length, 0);
        viewer.showAllObjects();
        assert.deepEqual(source, before);
        viewer.setObjectVisible(9, false);
        const removed = structuredClone(source);
        delete removed.objects[9];
        delete removed.faces[3];
        for (const id of [5, 6, 7, 8]) { delete removed.vertices[id]; }
        viewer.setMesh(removed);
        assert.equal(viewer.hiddenObjects.size, 0);
    } finally {
        viewer?.dispose();
        environment.restore();
    }
});

test('object inspector searches names, IDs and stages, hides matches, isolates and restores objects', () => {
    const environment = browserEnvironment();
    let viewer;
    let inspector;
    try {
        viewer = new ModelViewer(environment.document.getElementById('viewport'));
        const source = objectMesh();
        source.objects[9].stage = 'production_ready';
        viewer.setMesh(source);
        inspector = new ObjectInspector(viewer);
        inspector.setMesh(source);
        const search = environment.document.getElementById('objects-search');
        search.value = 'roof';
        search.dispatchEvent(new Event('input'));
        environment.document.getElementById('objects-hide-filtered').dispatchEvent(new Event('click'));
        assert.equal(viewer.isObjectVisible(2), false);
        assert.equal(viewer.isObjectVisible(9), true);
        search.value = 'production ready';
        search.dispatchEvent(new Event('input'));
        assert.equal(inspector.rows.get('9').matches, true);
        assert.equal(inspector.rows.get('2').matches, false);
        search.value = '9';
        search.dispatchEvent(new Event('input'));
        environment.document.getElementById('objects-isolate-filtered').dispatchEvent(new Event('click'));
        assert.equal(inspector.rows.get('9').appearances[0].checkbox.checked, true);
        assert.equal(inspector.rows.get('2').matches, false);
        environment.document.getElementById('objects-show-all').dispatchEvent(new Event('click'));
        assert.equal(viewer.hiddenObjects.size, 0);
        search.value = 'not present';
        search.dispatchEvent(new Event('input'));
        assert.equal(environment.document.getElementById('objects-empty').hidden, false);
    } finally {
        inspector?.dispose();
        viewer?.dispose();
        environment.restore();
    }
});

test('live geometry replacement preserves camera and disposes old GPU buffers', () => {
    const environment = browserEnvironment();
    let viewer = null;
    try {
        viewer = new ModelViewer(environment.document.getElementById('viewport'));
        viewer.setMesh(mesh);
        viewer.yaw = 0.7;
        const previousBuffers = [...viewer.surfaces.buffers, ...viewer.edges.buffers, ...viewer.ground.buffers];
        viewer.pitch = -0.4;
        viewer.target = [1, 2, 3];
        viewer.distance = 8;
        const edited = structuredClone(mesh);
        edited.vertices[3][1] = 2;
        viewer.setMesh(edited);
        assert.deepEqual(viewer.target, [1, 2, 3]);
        assert.equal(viewer.distance, 8);
        assert.equal(viewer.yaw, 0.7);
        assert.equal(viewer.pitch, -0.4);
        const deletedBuffers = environment.calls.filter(([name]) => name === 'deleteBuffer').map(call => call[1]);
        assert.deepEqual(deletedBuffers, previousBuffers);
        viewer.wireframe = true;
        viewer.pitch = 0.22;
        viewer.render();
        assert.ok(environment.calls.some(([name, mode]) => name === 'drawArrays' && mode === 'LINES'));
        const projections = environment.calls.filter(([name, uniform]) => name === 'uniformMatrix4fv' && uniform === 'projection');
        const modelProjection = projections.at(-1)[3];
        const groundProjection = projections.at(-2)[3];
        const modelNear = modelProjection[14] / (modelProjection[10] - 1);
        const foregroundDepth = modelNear * 0.5;
        const clipDepth = projection => (-projection[10] * foregroundDepth + projection[14]) / foregroundDepth;
        assert.ok(clipDepth(modelProjection) < -1, 'foreground lies in front of the tight model near plane');
        assert.ok(clipDepth(groundProjection) > -1 && clipDepth(groundProjection) < 1, 'the ground remains visible there');
        const camera = [...viewer.target, viewer.distance, viewer.yaw, viewer.pitch];
        for (const [mode, expected] of [['normals', 1], ['depth', 2], ['silhouette', 3], ['orientation', 4], ['face_ids', 5], ['object_ids', 6], ['uv_checker', 8], ['textured', 9], ['vertex_colors', 10], ['material', 11]]) {
            viewer.displayMode = mode;
            viewer.render();
            const uniforms = environment.calls.filter(([name, uniform]) => name === 'uniform1i' && uniform === 'surfaceMode');
            assert.equal(uniforms.at(-1)[2], expected);
        }
        assert.deepEqual([...viewer.target, viewer.distance, viewer.yaw, viewer.pitch], camera);
        viewer.frame();
        assert.deepEqual(viewer.target, viewer.bounds.center);
    } finally {
        viewer?.dispose();
        environment.restore();
    }
});

test('animated frames reuse geometry buffers and update positions, normals and edges', () => {
    const environment = browserEnvironment();
    let viewer;
    try {
        viewer = new ModelViewer(environment.document.getElementById('viewport'));
        const source = structuredClone(mesh);
        source.rig = {
            bones: [{ name: 'root', parent: null, position: [0, 0, 0] }],
            weights: Object.fromEntries(Object.keys(source.vertices).map(id => [id, [{ bone: 'root', weight: 1 }]])),
        };
        const clip = { tracks: { root: [
            { time: 0, rotation: [0, 0, 0], translation: [0, 0, 0] },
            { time: 1, rotation: [90, 0, 0], translation: [0, 0, 0] },
        ] } };
        const { samplePose, writeNormal, writeSurface } = createAnimationSampler(source, clip);
        viewer.setMesh(source);
        const surfaces = viewer.surfaces;
        environment.calls.length = 0;
        viewer.setPose(samplePose(1, true), writeNormal, writeSurface);
        assert.equal(environment.calls.filter(([name]) => name === 'bufferSubData').length, 2);
        viewer.wireframe = true;
        viewer.render();
        viewer.render();
        const uploads = environment.calls.filter(([name]) => name === 'bufferSubData');
        assert.equal(uploads.length, 3);
        assert.equal(environment.calls.some(([name]) => name === 'bufferData' || name === 'deleteBuffer'), false);
        assert.equal(viewer.surfaces, surfaces);
        assert.deepEqual(viewer.mesh.vertices, mesh.vertices);
        assert.ok(Math.abs(uploads[0][3][2] + Math.SQRT1_2) < 1e-6);
        assert.ok(Math.abs(uploads[1][3][1] + 1) < 1e-6);
        assert.ok(Math.abs(uploads[2][3][2] + Math.SQRT1_2) < 1e-6);
    } finally {
        viewer?.dispose();
        environment.restore();
    }
});

test('the rig overlay follows the sampled hierarchy and can draw without the mesh', () => {
    const environment = browserEnvironment();
    let viewer;
    try {
        viewer = new ModelViewer(environment.document.getElementById('viewport'));
        const source = structuredClone(mesh);
        source.rig = {
            bones: [
                { name: 'root', parent: null, position: [0, 0, 0] },
                { name: 'tip', parent: 'root', position: [1, 0, 0], tail: [2, 0, 0] },
            ],
            weights: {},
        };
        const clip = { tracks: {
            root: [{ time: 0, rotation: [0, 0, 0], translation: [0, 2, 0] }],
            tip: [{ time: 0, rotation: [0, 0, 90], translation: [0, 0, 0] }],
        } };
        const sampler = createAnimationSampler(source, clip);
        const display = { bones: true, hierarchy: true, joints: true, mesh: false };
        viewer.setMesh(source);
        viewer.setRigPose(null, display);
        const geometry = viewer.rigGeometry;
        sampler.samplePose(0);
        const transforms = sampler.boneTransforms();
        assert.deepEqual([...transforms.values()].map(joint => joint.origin), [[0, 2, 0], [1, 2, 0]]);

        environment.calls.length = 0;
        viewer.setRigPose(transforms, display);
        assert.equal(viewer.rigGeometry, geometry);
        const upload = environment.calls.find(([name]) => name === 'bufferSubData');
        const tail = [1, 3, 0].map((value, axis) => (value - viewer.origin[axis]) / viewer.scale);
        assert.deepEqual(upload[3].slice(9, 12), new Float32Array(tail));

        viewer.displayMode = 'normals';
        viewer.render();
        assert.deepEqual(environment.calls.filter(([name]) => name === 'drawArrays'), [
            ['drawArrays', 'LINES', 24, 2],
            ['drawArrays', 'LINES', 0, 24],
            ['drawArrays', 'POINTS', 26, 2],
        ]);
    } finally {
        viewer?.dispose();
        environment.restore();
    }
});

test('animation sampler applies track-level step and cubic timing', () => {
    const source = structuredClone(mesh);
    source.rig = {
        bones: [{ name: 'root', parent: null, position: [0, 0, 0] }],
        weights: { 1: [{ bone: 'root', weight: 1 }] },
    };
    const tracks = { root: [
        { time: 0, rotation: [0, 0, 0], translation: [0, 0, 0] },
        { time: 2, rotation: [0, 0, 0], translation: [0, 0, 0.4] },
    ] };
    for (const [mode, depth] of [['step', 0], ['linear', 0.1], ['cubic', 0.0625]]) {
        const { samplePose } = createAnimationSampler(source, { tracks, interpolation: { root: mode } });
        assert.equal(samplePose(0.5)[1][2], depth);
        assert.deepEqual(samplePose(0.5)[2], source.vertices[2]);
        assert.equal(samplePose(2)[1][2], 0.4);
    }
});

test('material maps bind independently and are released when appearance changes', () => {
    const environment = browserEnvironment();
    let viewer;
    try {
        viewer = new ModelViewer(environment.document.getElementById('viewport'));
        const source = structuredClone(mesh);
        const encode = pixel => Buffer.from(pixel).toString('base64');
        source.textures = { paint: { width: 1, height: 1, rgba_base64: encode([120, 90, 110, 255]), maps: { normal: encode([128, 128, 255, 255]), roughness: encode([180, 180, 180, 255]), metallic: encode([200, 200, 200, 255]), specular: encode([20, 20, 20, 255]) } } };
        source.face_textures = { 1: 'paint' };
        source.uvs = { 1: { 1: [0, 0], 2: [1, 0], 3: [1, 1], 4: [0, 1] } };
        viewer.setMesh(source);
        const previous = viewer.textures.get('paint');
        assert.equal(Object.keys(previous.maps).length, 4);
        viewer.displayMode = 'material';
        viewer.render();
        for (const uniform of ['hasNormal', 'hasRoughness', 'hasMetallic', 'hasSpecular']) {
            const values = environment.calls.filter(([name, key]) => name === 'uniform1i' && key === uniform);
            assert.equal(values.at(-1)[2], true);
        }
        source.textures.paint.maps = {};
        viewer.setMesh(source);
        const deleted = environment.calls.filter(([name]) => name === 'deleteTexture').map(call => call[1]);
        for (const handle of [previous.handle, ...Object.values(previous.maps)]) { assert.ok(deleted.includes(handle)); }
        viewer.render();
        const values = environment.calls.filter(([name, key]) => name === 'uniform1i' && key === 'hasNormal');
        assert.equal(values.at(-1)[2], false);
    } finally {
        viewer?.dispose();
        environment.restore();
    }
});

test('viewer polls read-only state, applies edits, skips selection-only uploads and reconnects', async () => {
    const environment = browserEnvironment();
    const timers = new Map();
    let nextTimer = 1;
    let state = { revision: 1, mesh: structuredClone(mesh) };
    let offline = false;
    const commands = [];
    environment.replace('setTimeout', (callback, delay) => {
        const id = nextTimer++;
        timers.set(id, { callback, delay });
        return id;
    });
    environment.replace('clearTimeout', id => timers.delete(id));
    environment.replace('fetch', async (_, options) => {
        commands.push(_); 
        if (offline === true) { throw new TypeError('Offline'); }
        if (_ === '/workspaces') { return { json: async () => ({ ok: true, result: { mode: 'single' } }) }; }
        return { json: async () => ({ ok: true, result: state, workspace: '\\\\?\\C:\\models\\shader-ball' }) };
    });
    const tick = async () => {
        const entry = [...timers.entries()].find(([, timer]) => timer.delay === 400);
        assert.ok(entry, 'one refresh is scheduled after the previous request completes');
        timers.delete(entry[0]);
        await entry[1].callback();
    };
    try {
        await import('../web/app.js');
        await new Promise(setImmediate);
        const uploads = () => environment.calls.filter(([name]) => name === 'bufferData').length;
        const initialUploads = uploads();
        assert.equal(environment.document.getElementById('connection').textContent, 'Live');
        assert.equal(environment.document.getElementById('workspace-path').textContent, 'C:\\models\\shader-ball');
        state = { revision: 2, mesh: { ...mesh, selection: [1] } };
        await tick();
        assert.equal(uploads(), initialUploads);
        state = { revision: 3, mesh: structuredClone(mesh) };
        state.mesh.vertices[3][1] = 2;
        await tick();
        assert.equal(uploads(), initialUploads * 2);
        offline = true;
        await tick();
        assert.equal(environment.document.getElementById('connection').textContent, 'Reconnecting');
        assert.equal(uploads(), initialUploads * 2);
        offline = false;
        state = { revision: 3, mesh: structuredClone(mesh) };
        await tick();
        assert.equal(uploads(), initialUploads * 3, 'same revision after a server restart still checks geometry');
        state.mesh.flat_faces = [1];
        await tick();
        assert.equal(uploads(), initialUploads * 4, 'region shading updates GPU normals without a position or topology change');
        state.mesh.uvs = { 1: { 1: [0, 0], 2: [1, 0], 3: [1, 1], 4: [0, 1] } };
        await tick();
        assert.equal(uploads(), initialUploads * 5, 'UV edits refresh the live preview');
        state.mesh.vertex_colors = { 1: [1, 0, 0] };
        await tick();
        assert.equal(uploads(), initialUploads * 6, 'painting refreshes without geometry changes');
        state.mesh.textures = { test: { width: 1, height: 1, rgba_base64: '/wAA/w==' } };
        state.mesh.face_textures = { 1: 'test' };
        await tick();
        assert.equal(uploads(), initialUploads * 7, 'texture assignment refreshes without geometry changes');
        assert.equal(environment.document.getElementById('notice').hidden, true);
        assert.equal(commands[0], '/workspaces');
        assert.ok(commands.slice(1).every(path => path === '/state'));
        environment.window.dispatchEvent(new Event('pagehide'));
        assert.equal(timers.size, 0);
    } finally {
        environment.restore();
    }
});


test('project viewer scopes selection and polls saved revisions without writing', async () => {
    const environment = browserEnvironment();
    const url = new URL('http://localhost/?workspace=models%2Ffin');
    const timers = new Map();
    const requests = [];
    let nextTimer = 1;
    let savedRevision = 7;
    let savedReads = 0;
    let destination;
    url.assign = next => { destination = next; };
    environment.window.location = url;
    environment.replace('setTimeout', (callback, delay) => {
        const id = nextTimer++;
        timers.set(id, { callback, delay });
        return id;
    });
    environment.replace('clearTimeout', id => timers.delete(id));
    environment.replace('fetch', async (path, options) => {
        requests.push([path, options.method ?? 'GET']);
        let data = { ok: true };
        switch (path) {
            case '/workspaces':
                data.result = { mode: 'viewer', workspaces: [{ path: 'models/fin' }, { path: 'models/nose' }] };
                break;
            case '/state/meta?workspace=models%2Ffin':
                data.result = { tag: `W/"saved-${savedRevision}"` };
                break;
            case '/state?workspace=models%2Ffin': {
                const tag = `W/"saved-${savedRevision}"`;
                const headers = new Headers({ ETag: tag });
                if (options.headers?.['If-None-Match'] === tag) {
                    return { status: 304, headers };
                }
                savedReads++;
                data.result = { revision: savedRevision, mesh };
                return { status: 200, headers, json: async () => data };
            }
            default:
                assert.fail(`unexpected request ${path}`);
        }
        return { ok: true, json: async () => data };
    });
    const element = id => environment.document.getElementById(id);
    const tick = async () => {
        const [id, timer] = [...timers.entries()].find(([, timer]) => timer.delay === 400);
        timers.delete(id);
        await timer.callback();
    };
    try {
        await import('../web/app.js?project');
        await new Promise(setImmediate);
        assert.equal(element('workspace-path').textContent, 'models/fin');
        assert.equal(element('revision').textContent, 'rev 7');
        assert.equal(element('model-name').textContent, 'Saved model');
        await tick();
        assert.equal(savedReads, 1, 'unchanged polls do not fetch another mesh payload');
        assert.equal(requests.filter(([path]) => path === '/state?workspace=models%2Ffin').length, 1);
        const picker = element('workspace-picker');
        const shortcut = new Event('keydown', { cancelable: true });
        Object.assign(shortcut, { key: 'p', ctrlKey: true });
        environment.document.dispatchEvent(shortcut);
        assert.equal(shortcut.defaultPrevented, true);
        assert.equal(picker.open, true);
        const search = element('workspace-search');
        search.value = 'm nse';
        search.dispatchEvent(new Event('input'));
        assert.deepEqual(element('workspace-results').children.map(option => option.textContent), ['models/nose']);
        const enter = new Event('keydown', { cancelable: true });
        Object.assign(enter, { key: 'Enter' });
        search.dispatchEvent(enter);
        assert.equal(destination.searchParams.get('workspace'), 'models/nose');
        picker.close();

        savedRevision = 8;
        await tick();
        assert.equal(element('revision').textContent, 'rev 8');
        assert.equal(element('model-name').textContent, 'Saved model');
        assert.ok(requests.every(([, method]) => method === 'GET'));
    } finally {
        environment.window.dispatchEvent(new Event('pagehide'));
        environment.restore();
    }
});

test('workspace views restore after switching without leaking into other assets', async () => {
    const storage = new Map();
    const setMesh = ModelViewer.prototype.setMesh;
    let viewer;
    ModelViewer.prototype.setMesh = function (mesh) {
        viewer = this;
        return setMesh.call(this, mesh);
    };
    async function openWorkspace(asset, visit, inspect) {
        const environment = browserEnvironment();
        const query = new URLSearchParams({ workspace: asset });
        environment.window.location = new URL(`http://localhost/?${query}`);
        environment.window.localStorage = {
            getItem: key => storage.get(key) ?? null,
            setItem: (key, value) => storage.set(key, value),
        };
        environment.document.getElementById('objects-panel').hidden = true;
        environment.replace('fetch', async path => {
            const result = path === '/workspaces' ? { mode: 'viewer', workspaces: [] }
                : path.startsWith('/state/meta') ? { tag: asset } : { mesh: objectMesh(), revision: 1 };
            return { json: async () => ({ ok: true, result, workspace: `C:/models/${asset}` }) };
        });
        try {
            await import(`../web/app.js?workspace-view-test=${visit}`);
            await new Promise(setImmediate);
            inspect(environment.document);
        } finally {
            environment.window.dispatchEvent(new Event('pagehide'));
            environment.restore();
        }
    }

    const camera = { target: [1, 2, 3], distance: 5, yaw: 1.2, pitch: 0.3 };
    try {
        await openWorkspace('helmet', 1, document => {
            viewer.restoreCamera(camera);
            viewer.setObjectVisible(2, false);
            document.getElementById('grid').dispatchEvent(new Event('click'));
        });
        await openWorkspace('cylinder', 2, () => {
            assert.equal(viewer.grid, false);
            assert.equal(viewer.isObjectVisible(2), true);
            assert.notEqual(viewer.yaw, camera.yaw);
        });
        await openWorkspace('helmet', 3, () => {
            assert.equal(viewer.grid, true);
            assert.equal(viewer.isObjectVisible(2), false);
            assert.equal(viewer.yaw, camera.yaw);
            assert.equal(viewer.pitch, camera.pitch);
            assert.ok(Math.abs(viewer.saveCamera().distance - camera.distance) < 1e-12);
            assert.ok(viewer.saveCamera().target.every((value, axis) => Math.abs(value - camera.target[axis]) < 1e-12));
        });
        assert.equal(storage.size, 1);
    } finally {
        ModelViewer.prototype.setMesh = setMesh;
    }
});

test('replay shows saved states and failed command detail without writing to the live model', async () => {
    const environment = browserEnvironment();
    const requests = [];
    const shown = [];
    let live = false;
    const events = [
        { id: 1, session: '1000000-1', source_event: 1, command: 'select', ok: true, snapshot: 1, response_bytes: 50, duration_ms: 1 },
        { id: 2, session: '2000000-1', source_event: 1, command: 'move', ok: false, snapshot: 0, response_bytes: 30, duration_ms: 2 }
    ];
    const states = { 0: { revision: 0, mesh }, 1: { revision: 1, mesh: { ...mesh, selection: [1] } } };
    environment.replace('fetch', async (path, options) => {
        requests.push([path, options]);
        let data;
        if (path === '/sessions') { data = { current: '2000000-1', timeline: 'workspace', sessions: ['2000000-1', '1000000-1'] }; }
        else if (path === '/sessions/workspace') { data = { events, initial_session: '1000000-1' }; }
        else if (path.includes('/states/')) { data = states[path === '/sessions/1000000-1/states/0' ? 0 : 1]; }
        else { data = { input: { command: 'move' }, output: { ok: false, error: 'invalid movement' } }; }
        return { ok: true, json: async () => data };
    });
    let replay;
    try {
        replay = new SessionReplay(state => shown.push(state), () => { live = true; });
        await replay.open();
        assert.equal(replay.active, true);
        assert.equal(replay.session, 'workspace', 'reopening shows construction across server restarts');
        assert.deepEqual(shown.at(-1), states[0]);
        await replay.seek(2);
        assert.deepEqual(shown.at(-1), states[1]);
        replay.hide();
        assert.equal(replay.active, true, 'opening the object inspector can hide replay controls without returning to live geometry');
        assert.equal(environment.document.getElementById('replay-panel').hidden, true);
        assert.equal(requests.at(-2)[0], '/sessions/2000000-1/states/0');
        assert.equal(requests.at(-1)[0], '/sessions/2000000-1/events/1');
        assert.deepEqual(JSON.parse(environment.document.getElementById('replay-detail').textContent), { command: 'move' });
        environment.document.getElementById('replay-output-tab').dispatchEvent(new Event('click'));
        assert.match(environment.document.getElementById('replay-detail').textContent, /invalid movement/);
        assert.ok(requests.every(([, options]) => options.method === undefined));
        replay.live();
        assert.equal(replay.active, false);
        assert.equal(live, true);
    } finally {
        replay?.dispose();
        environment.restore();
    }
});

test('a missing replay snapshot leaves the latest model active', async () => {
    const environment = browserEnvironment();
    const shown = [];
    const latest = environment.document.getElementById('return-to-latest');
    latest.hidden = true;
    environment.replace('fetch', async path => {
        if (path.endsWith('/states/0')) { return { ok: false, json: async () => ({ error: 'Missing snapshot' }) }; }
        return { ok: true, json: async () => ({ events: [] }) };
    });
    const replay = new SessionReplay(state => shown.push(state), () => {});
    try {
        await replay.load('1000000-1');
        assert.equal(replay.active, false);
        assert.equal(latest.hidden, true);
        assert.equal(shown.length, 0);
        assert.equal(environment.document.getElementById('replay-summary').textContent, 'Missing snapshot');
    } finally {
        replay.dispose();
        environment.restore();
    }
});


test('collections share a primary colour and overlapping visibility stays in sync', () => {
    const environment = browserEnvironment();
    let viewer = null;
    let inspector = null;
    try {
        const source = objectMesh();
        source.collections = { 3: { name: 'Structure', objects: [2, 9] }, 7: { name: 'Ceiling', objects: [2] } };
        source.primary_collections = { 2: 3, 9: 3 };
        const before = structuredClone(source);
        const data = buildMeshData(source, [0, 0, 0], 1);
        const expected = new Float32Array(facePalette(2)[0].map(value => value / 255));
        for (let index = 0; index < data.collectionColors.length; index += 3) {
            assert.deepEqual(data.collectionColors.slice(index, index + 3), expected);
        }
        viewer = new ModelViewer(environment.document.getElementById('viewport'));
        viewer.setMesh(source);
        inspector = new ObjectInspector(viewer);
        inspector.setMesh(source);
        const uploads = environment.calls.filter(([name]) => name === 'bufferData').length;
        const ceiling = inspector.collectionRows[1];
        ceiling.checkbox.checked = false;
        ceiling.checkbox.dispatchEvent(new Event('change'));
        assert.equal(viewer.isObjectVisible(2), false);
        assert.equal(viewer.isObjectVisible(9), true);
        assert.equal(inspector.collectionRows[0].checkbox.indeterminate, true);
        ceiling.solo.dispatchEvent(new Event('click'));
        assert.equal(viewer.isObjectVisible(2), true);
        assert.equal(viewer.isObjectVisible(9), false);
        assert.equal(environment.calls.filter(([name]) => name === 'bufferData').length, uploads);
        const search = environment.document.getElementById('objects-search');
        search.value = 'ceiling';
        search.dispatchEvent(new Event('input'));
        assert.equal(inspector.rows.get('2').matches, true);
        assert.equal(inspector.rows.get('9').matches, false);
        assert.deepEqual(source, before);
        source.primary_collections = {};
        const unassigned = buildMeshData(source, [0, 0, 0], 1);
        assert.equal(unassigned.collectionColors[0], Math.fround(128 / 255));
    } finally {
        inspector?.dispose();
        viewer?.dispose();
        environment.restore();
    }
});


test('collection branches preserve expansion, reveal search results and sync duplicate rows', () => {
    const environment = browserEnvironment();
    let viewer = null;
    let inspector = null;
    try {
        const source = objectMesh();
        source.collections = { 1: { name: 'Structure', objects: [2, 9] }, 2: { name: 'Ceiling', objects: [2] }, 3: { name: 'Empty', objects: [] } };
        const before = structuredClone(source);
        viewer = new ModelViewer(environment.document.getElementById('viewport'));
        viewer.setMesh(source);
        inspector = new ObjectInspector(viewer);
        inspector.setMesh(source);
        assert.equal(inspector.rows.get('2').appearances.length, 2);
        assert.equal(inspector.rows.get('9').appearances.length, 1);
        assert.equal(inspector.collectionRows[0].branch.open, false);
        assert.equal(inspector.collectionRows[2].checkbox.disabled, true);
        inspector.collectionRows[0].branch.open = true;
        inspector.collectionRows[0].branch.dispatchEvent(new Event('toggle'));
        inspector.setMesh(source);
        assert.equal(inspector.collectionRows[0].branch.open, true, 'expansion survives live updates and replay snapshots');
        const search = environment.document.getElementById('objects-search');
        search.value = 'ceiling';
        search.dispatchEvent(new Event('input'));
        assert.equal(inspector.collectionRows[0].branch.hidden, true);
        assert.equal(inspector.collectionRows[1].branch.open, true);
        assert.equal(inspector.rows.get('2').matches, true);
        assert.equal(inspector.rows.get('9').matches, false);
        assert.match(environment.document.getElementById('objects-summary').textContent, /1 matching/);
        environment.document.getElementById('objects-hide-filtered').dispatchEvent(new Event('click'));
        assert.ok(inspector.rows.get('2').appearances.every(row => row.checkbox.checked === false));
        assert.equal(inspector.collectionRows[0].checkbox.indeterminate, true);
        search.value = '';
        search.dispatchEvent(new Event('input'));
        assert.equal(inspector.collectionRows[0].branch.open, true);
        assert.equal(inspector.collectionRows[1].branch.open, false, 'search does not replace the saved expansion choice');
        assert.match(environment.document.getElementById('objects-summary').textContent, /2 matching/);
        const copy = inspector.rows.get('2').appearances[1];
        copy.checkbox.checked = true;
        copy.checkbox.dispatchEvent(new Event('change'));
        assert.ok(inspector.rows.get('2').appearances.every(row => row.checkbox.checked === true));
        inspector.resetExpansion();
        inspector.setMesh(source);
        assert.ok(inspector.collectionRows.every(row => row.branch.open === false));
        const ungrouped = structuredClone(source);
        ungrouped.collections = { 1: { name: 'Ceiling', objects: [2] } };
        inspector.setMesh(ungrouped);
        assert.equal(inspector.collectionRows[1].id, 'ungrouped');
        assert.deepEqual(inspector.collectionRows[1].members, ['9']);
        assert.equal(inspector.rows.get('9').appearances.length, 1);
        assert.deepEqual(source, before);
    } finally {
        inspector?.dispose();
        viewer?.dispose();
        environment.restore();
    }
});

test('rotated nonuniform instance stretches preserve inverse-transpose smooth normals', () => {
    const vertices = { ...mesh.vertices, 5: [1, 1, -2], 6: [-1, 1, -2] };
    const faces = { 1: [1,2,3,4], 2: [4,3,5,6] };
    const source = Object.keys(vertices).map(Number);
    for (const id of source) {
        const [x,y,z] = vertices[id];
        vertices[id+6] = [-y*2,x,z*4];
    }
    faces[3] = faces[1].map(id => id+6);
    faces[4] = faces[2].map(id => id+6);
    const instance = { master: 1, scale: [1,2,4], rotation: [[0,-1,0],[1,0,0],[0,0,1]], faces: {1:3,2:4} };
    const objects = { 1: { vertices:source, smooth_angle:120 }, 2:{ vertices:source.map(id => id+6), smooth_angle:120, instance } };
    const actual = buildCornerNormals({vertices, faces, objects})[2][2];
    const expected = [-2/Math.sqrt(5), 0, 1/Math.sqrt(5)];
    for (let axis=0; axis<3; axis++) { assert.ok(Math.abs(actual[axis]-expected[axis]) < 1e-10); }
});
