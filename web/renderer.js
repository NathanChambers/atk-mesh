import { buildMeshData, cameraMatrix, measureMesh, orthographic, perspective } from './geometry.js';
import { boneDisplayPoints } from './animation.js';

const vertexSource = `#version 300 es
in vec3 position;
in vec3 normal;
in vec3 faceColor;
in vec3 objectColor;
in vec3 collectionColor;
in vec3 uvCoordinate;
in vec3 vertexColor;
uniform mat4 projection;
uniform mat4 camera;
out vec3 surfaceNormal;
out vec3 worldPosition;
out vec3 surfaceColor;
out vec3 surfaceObjectColor;
out vec3 surfaceCollectionColor;
out vec3 surfaceUv;
out vec3 surfacePaint;
void main() {
    surfaceNormal = normal;
    worldPosition = position;
    surfaceColor = faceColor;
    surfaceObjectColor = objectColor;
    surfaceCollectionColor = collectionColor;
    surfaceUv = uvCoordinate;
    surfacePaint = vertexColor;
    gl_Position = projection * camera * vec4(position, 1.0);
    gl_PointSize = 8.0;
}`;

const fragmentSource = `#version 300 es
precision highp float;
in vec3 surfaceNormal;
in vec3 worldPosition;
in vec3 surfaceColor;
in vec3 surfaceObjectColor;
in vec3 surfaceCollectionColor;
in vec3 surfaceUv;
in vec3 surfacePaint;
uniform sampler2D textureImage;
uniform bool hasTexture;
uniform float alphaCutoff;
uniform sampler2D normalImage;
uniform sampler2D roughnessImage;
uniform sampler2D metallicImage;
uniform sampler2D specularImage;
uniform bool hasNormal;
uniform bool hasRoughness;
uniform bool hasMetallic;
uniform bool hasSpecular;
uniform vec3 eye;
uniform int mode;
uniform bool showGrid;
uniform vec3 groundCenter;
uniform float groundRadius;
uniform int surfaceMode;
uniform vec3 towardCamera;
uniform vec2 depthRange;
out vec4 color;
void main() {
    vec3 background = vec3(0.090, 0.102, 0.125);
    if (mode == 3) {
        color = vec4(0.35, 0.82, 1.0, 1.0);
        return;
    }
    if (mode == 4) {
        if (length(gl_PointCoord - vec2(0.5)) > 0.5) { discard; }
        color = vec4(1.0, 0.76, 0.35, 1.0);
        return;
    }
    if (mode == 5) {
        color = vec4(0.49, 0.57, 0.68, 1.0);
        return;
    }
    if (mode == 2) {
        color = vec4(0.12, 0.17, 0.22, 1.0);
        return;
    }
    if (mode == 1) {
        vec2 offset = worldPosition.xz - groundCenter.xz;
        float distanceFromModel = length(offset) / groundRadius;
        float contact = exp(-dot(offset, offset) / (groundRadius * groundRadius * 0.24));
        vec3 ground = background + vec3(0.012) * exp(-distanceFromModel * 0.3);
        ground *= 1.0 - contact * 0.48;
        if (showGrid) {
            vec2 coordinates = worldPosition.xz / (groundRadius * 0.25);
            vec2 line = abs(fract(coordinates - 0.5) - 0.5) / max(fwidth(coordinates), vec2(0.0001));
            float grid = 1.0 - min(min(line.x, line.y), 1.0);
            ground += vec3(grid * 0.045 * exp(-distanceFromModel * 0.3));
        }
        float fade = exp(-distanceFromModel * distanceFromModel * 0.025);
        color = vec4(mix(background, ground, fade), 1.0);
        return;
    }
    vec3 n = normalize(surfaceNormal);
    if (surfaceMode == 11) {
        if (surfaceUv.z < 0.5) { color = vec4(1.0, 0.0, 1.0, 1.0); return; }
        vec2 coordinates = vec2(fract(surfaceUv.x), 1.0 - fract(surfaceUv.y));
        vec4 texel = hasTexture ? texture(textureImage, coordinates) : vec4(vec3(0.5), 1.0);
        vec3 base = pow(mix(vec3(128.0 / 255.0), texel.rgb, alphaCutoff >= 0.0 ? 1.0 : texel.a) * surfacePaint, vec3(2.2));
        float roughness = clamp(hasRoughness ? texture(roughnessImage, coordinates).r : 0.65, 0.04, 1.0);
        float metallic = hasMetallic ? texture(metallicImage, coordinates).r : 0.0;
        vec3 specular = hasSpecular ? texture(specularImage, coordinates).rgb : vec3(0.04);
        vec3 dpdx = dFdx(worldPosition);
        vec3 dpdy = dFdy(worldPosition);
        vec2 duvdx = dFdx(surfaceUv.xy);
        vec2 duvdy = dFdy(surfaceUv.xy);
        float determinant = duvdx.x * duvdy.y - duvdx.y * duvdy.x;
        if (hasNormal && abs(determinant) > 0.000000000001) {
            vec3 rawTangent = (dpdx * duvdy.y - dpdy * duvdx.y) / determinant;
            vec3 rawBitangent = (dpdy * duvdx.x - dpdx * duvdy.x) / determinant;
            vec3 tangent = normalize(rawTangent - n * dot(n, rawTangent));
            float handedness = dot(cross(n, tangent), rawBitangent) < 0.0 ? -1.0 : 1.0;
            vec3 bitangent = cross(n, tangent) * handedness;
            vec3 mapped = normalize(texture(normalImage, coordinates).xyz * 2.0 - 1.0);
            n = normalize(tangent * mapped.x + bitangent * mapped.y + n * mapped.z);
        }
        vec3 viewDirection = normalize(eye - worldPosition);
        if (dot(surfaceNormal, viewDirection) < 0.0) { n = -n; }
        vec3 lights[3] = vec3[3](normalize(vec3(-3.0, 5.0, 4.0)), normalize(vec3(4.0, 2.0, 1.0)), normalize(vec3(1.0, 3.0, -4.0)));
        float weights[3] = float[3](0.8, 0.27, 0.38);
        float diffuse = 0.0;
        float highlight = 0.0;
        float exponent = clamp(2.0 / (roughness * roughness) - 2.0, 2.0, 512.0);
        for (int index = 0; index < 3; index++) {
            float facing = max(dot(n, lights[index]), 0.0);
            diffuse += facing * weights[index];
            if (facing > 0.0) { highlight += pow(max(dot(n, normalize(lights[index] + viewDirection)), 0.0), exponent) * weights[index] * (0.5 + 1.5 * (1.0 - roughness)); }
        }
        float ambient = 0.20 + 0.15 * (n.y * 0.5 + 0.5);
        vec3 f0 = mix(specular, base, metallic);
        vec3 linearColor = base * (ambient + (1.0 - metallic) * diffuse) + f0 * highlight;
        if (texel.a < alphaCutoff) { discard; }
        color = vec4(pow(linearColor / (linearColor + vec3(0.65)), vec3(1.0 / 2.2)), 1.0);
        return;
    }
    if (surfaceMode == 10) { color = vec4(surfacePaint, 1.0); return; }
    if (surfaceMode == 8 || surfaceMode == 9) {
        if (surfaceUv.z < 0.5) { color = vec4(1.0, 0.0, 1.0, 1.0); return; }
        if (surfaceMode == 8) {
            float odd = mod(floor(surfaceUv.x * 8.0) + floor(surfaceUv.y * 8.0), 2.0);
            color = vec4(vec3(odd > 0.5 ? 48.0 / 255.0 : 224.0 / 255.0), 1.0);
        } else if (hasTexture) {
            vec4 texel = texture(textureImage, vec2(fract(surfaceUv.x), 1.0 - fract(surfaceUv.y)));
            if (texel.a < alphaCutoff) { discard; }
            color = vec4(mix(vec3(128.0 / 255.0), texel.rgb, alphaCutoff >= 0.0 ? 1.0 : texel.a) * surfacePaint, 1.0);
        } else { color = vec4(vec3(128.0 / 255.0), 1.0); }
        return;
    }
    if (surfaceMode == 1) {
        color = vec4(n * 0.5 + 0.5, 1.0);
        return;
    }
    if (surfaceMode == 2) {
        float span = depthRange.y - depthRange.x;
        float fraction = span > 0.0000001 ? clamp((dot(worldPosition, towardCamera) - depthRange.x) / span, 0.0, 1.0) : 0.5;
        color = vec4(vec3((32.0 + 223.0 * fraction) / 255.0), 1.0);
        return;
    }
    if (surfaceMode == 3) { color = vec4(0.0, 0.0, 0.0, 1.0); return; }
    if (surfaceMode == 4) { color = gl_FrontFacing ? vec4(vec3(180.0 / 255.0), 1.0) : vec4(220.0 / 255.0, 55.0 / 255.0, 40.0 / 255.0, 1.0); return; }
    if (surfaceMode == 5) { color = vec4(surfaceColor, 1.0); return; }
    if (surfaceMode == 7) { color = vec4(surfaceCollectionColor, 1.0); return; }
    if (surfaceMode == 6) { color = vec4(surfaceObjectColor, 1.0); return; }
    if (!gl_FrontFacing) { n = -n; }
    vec3 viewDirection = normalize(eye - worldPosition);
    vec3 keyDirection = normalize(vec3(-3.0, 5.0, 4.0));
    vec3 fillDirection = normalize(vec3(4.0, 2.0, 1.0));
    vec3 rimDirection = normalize(vec3(1.0, 3.0, -4.0));
    float key = max(dot(n, keyDirection), 0.0);
    float fill = max(dot(n, fillDirection), 0.0);
    float rim = max(dot(n, rimDirection), 0.0);
    vec3 ambient = vec3(0.12 + 0.20 * (n.y * 0.5 + 0.5));
    vec3 light = ambient + vec3(key * 0.8 + fill * 0.27 + rim * 0.38);
    float specular = pow(max(dot(n, normalize(keyDirection + viewDirection)), 0.0), 42.0) * 0.09;
    vec3 base = surfaceMode == 12 ? pow(surfacePaint, vec3(2.2)) * 0.45 : vec3(0.45);
    vec3 linearColor = base * light + specular;
    vec3 mapped = linearColor / (linearColor + vec3(0.65));
    color = vec4(pow(mapped, vec3(1.0 / 2.2)), 1.0);
}`;

export class ModelViewer {
    constructor(canvas) {
        this.canvas = canvas;
        this.gl = canvas.getContext('webgl2', { antialias: true, alpha: false });
        if (this.gl === null) { throw new Error('WebGL 2 is unavailable. Enable hardware acceleration or use a browser with WebGL 2 support.'); }

        this.program = this.createProgram();
        this.locations = {};
        for (const name of ['projection', 'camera', 'eye', 'mode', 'showGrid', 'groundCenter', 'groundRadius', 'surfaceMode', 'towardCamera', 'depthRange', 'textureImage', 'hasTexture', 'alphaCutoff', 'normalImage', 'roughnessImage', 'metallicImage', 'specularImage', 'hasNormal', 'hasRoughness', 'hasMetallic', 'hasSpecular']) {
            this.locations[name] = this.gl.getUniformLocation(this.program, name);
        }
        this.mesh = null;
        this.origin = null;
        this.scale = 1;
        this.bounds = null;
        this.surfaces = null;
        this.edges = null;
        this.ground = null;
        this.rigGeometry = null;
        this.rigLineCount = 0;
        this.rigHierarchyCount = 0;
        this.rigJointCount = 0;
        this.showMesh = true;
        this.target = [0, 0, 0];
        this.distance = 3.5;
        this.yaw = Math.PI / 4;
        this.pitch = 0.22;
        this.orthographic = false;
        this.grid = false;
        this.turntable = false;
        this.wireframe = false;
        this.displayMode = 'shaded';
        this.depthPoints = [];
        this.posedVertices = null;
        this.poseEdgesDirty = false;
        this.hiddenObjects = new Set();
        this.groups = [];
        this.textures = new Map();
        this.fallbackTexture = this.createTexture(1, 1, new Uint8Array([255, 255, 255, 255]));
        this.vertexOwners = new Map();
        this.dirty = true;
        this.disposed = false;
        this.contextLost = false;
        this.pointers = new Map();
        this.lastTime = null;
        this.animation = null;
        this.events = new AbortController();
        this.bindControls();
        this.resizeObserver = new ResizeObserver(() => { this.dirty = true; });
        this.resizeObserver.observe(canvas);
        this.animate = this.animate.bind(this);
        this.animation = requestAnimationFrame(this.animate);
    }

    dispose() {
        this.disposed = true;
        cancelAnimationFrame(this.animation);
        this.events.abort();
        this.resizeObserver.disconnect();
        this.deleteGeometry(this.surfaces);
        this.deleteGeometry(this.edges);
        this.deleteGeometry(this.ground);
        this.deleteGeometry(this.rigGeometry);
        this.gl.deleteProgram(this.program);
        for (const texture of this.textures.values()) { this.deleteTexture(texture); }
        this.gl.deleteTexture(this.fallbackTexture);
    }

    createProgram() {
        const gl = this.gl;
        const shaders = [];
        for (const [type, source] of [[gl.VERTEX_SHADER, vertexSource], [gl.FRAGMENT_SHADER, fragmentSource]]) {
            const shader = gl.createShader(type);
            gl.shaderSource(shader, source);
            gl.compileShader(shader);
            if (gl.getShaderParameter(shader, gl.COMPILE_STATUS) === false) { throw new Error(gl.getShaderInfoLog(shader)); }
            shaders.push(shader);
        }
        const program = gl.createProgram();
        for (const shader of shaders) { gl.attachShader(program, shader); }
        gl.linkProgram(program);
        for (const shader of shaders) { gl.deleteShader(shader); }
        if (gl.getProgramParameter(program, gl.LINK_STATUS) === false) { throw new Error(gl.getProgramInfoLog(program)); }

        return program;
    }

    createGeometry(positions, normals = null, faceColors = null, objectColors = null, collectionColors = null, uvCoordinates = null, vertexColors = null) {
        const gl = this.gl;
        const vertexArray = gl.createVertexArray();
        gl.bindVertexArray(vertexArray);
        const buffers = [];
        for (const [name, data] of [['position', positions], ['normal', normals], ['faceColor', faceColors], ['objectColor', objectColors], ['collectionColor', collectionColors], ['uvCoordinate', uvCoordinates], ['vertexColor', vertexColors]]) {
            const attribute = gl.getAttribLocation(this.program, name);
            if (data === null) { gl.disableVertexAttribArray(attribute); gl.vertexAttrib3f(attribute, 0, 1, 0); continue; }

            const buffer = gl.createBuffer();
            gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
            gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
            gl.enableVertexAttribArray(attribute);
            gl.vertexAttribPointer(attribute, 3, gl.FLOAT, false, 0, 0);
            buffers.push(buffer);
        }
        gl.bindVertexArray(null);
        return { vertexArray, buffers, count: positions.length / 3 };
    }

    deleteGeometry(geometry) {
        if (geometry === null) { return; }

        for (const buffer of geometry.buffers) { this.gl.deleteBuffer(buffer); }
        this.gl.deleteVertexArray(geometry.vertexArray);
    }

    createTexture(width, height, pixels) {
        const gl = this.gl;
        const texture = gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
        return texture;
    }

    syncTextures(mesh) {
        for (const [name, texture] of this.textures) {
            if (JSON.stringify(mesh.textures?.[name]) !== texture.key) {
                this.deleteTexture(texture);
                this.textures.delete(name);
            }
        }
        for (const [name, texture] of Object.entries(mesh.textures || {})) {
            if (this.textures.has(name) === true) { continue; }

            const bytes = Uint8Array.from(atob(texture.rgba_base64), character => character.charCodeAt(0));
            const handle = this.createTexture(texture.width, texture.height, bytes);
            const maps = {};
            for (const channel of ['normal', 'roughness', 'metallic', 'specular']) {
                if (texture.maps?.[channel] === undefined) { continue; }
                const pixels = Uint8Array.from(atob(texture.maps[channel]), character => character.charCodeAt(0));
                maps[channel] = this.createTexture(texture.width, texture.height, pixels);
            }
            this.textures.set(name, { handle, maps, alphaCutoff: texture.alpha_cutoff ?? -1, key: JSON.stringify(texture) });
        }
    }

    deleteTexture(texture) {
        this.gl.deleteTexture(texture.handle);
        for (const handle of Object.values(texture.maps || {})) { this.gl.deleteTexture(handle); }
    }

    setMesh(mesh) {
        const bounds = measureMesh(mesh);
        const firstModel = this.origin === null;
        if (firstModel === true) {
            this.origin = bounds.center;
            this.scale = bounds.radius;
        }
        const data = buildMeshData(mesh, this.origin, this.scale);
        const surfaces = this.createGeometry(data.positions, data.normals, data.faceColors, data.objectColors, data.collectionColors, data.uvCoordinates, data.vertexColors);
        const edges = this.createGeometry(data.edges);
        this.deleteGeometry(this.surfaces);
        this.deleteGeometry(this.edges);
        this.surfaces = surfaces;
        this.edges = edges;
        this.groups = data.groups;
        this.depthPoints = Object.entries(mesh.vertices).map(([id, point]) => ({ id, object: data.owners.get(id), point: point.map((value, axis) => (value - this.origin[axis]) / this.scale) }));
        this.dirty = true;
        this.poseBuffers = data.poseSources === null ? null : {
            sources: data.poseSources,
            positions: data.positions,
            normals: data.normals.slice(),
            restNormals: data.normals,
            edges: data.edges,
        };
        this.posedVertices = null;
        this.poseEdgesDirty = false;

        this.syncTextures(mesh);
        const center = bounds.center.map((value, axis) => (value - this.origin[axis]) / this.scale);
        const groundY = (bounds.minimum[1] - this.origin[1]) / this.scale - 0.002;
        const radius = bounds.radius / this.scale;
        const size = radius * 40;
        const x = center[0];
        const z = center[2];
        const groundPositions = new Float32Array([x-size, groundY, z-size, x-size, groundY, z+size, x+size, groundY, z+size, x-size, groundY, z-size, x+size, groundY, z+size, x+size, groundY, z-size]);
        const ground = this.createGeometry(groundPositions);

        this.deleteGeometry(this.ground);
        this.ground = ground;
        this.mesh = mesh;
        this.vertexOwners = data.owners;
        this.hiddenObjects = new Set([...this.hiddenObjects].filter(id => Object.hasOwn(mesh.objects || {}, id)));
        this.bounds = { center, radius, groundY };
        if (firstModel === true) { this.frame(); }
    }

    setPose(vertices, writeNormal, writeSurface) {
        if (this.poseBuffers === null) { return; }

        const { sources, positions, normals, restNormals } = this.poseBuffers;
        if (writeSurface !== undefined) {
            writeSurface(sources.surface, restNormals, positions, normals, this.origin, this.scale);
        } else {
            const [originX, originY, originZ] = this.origin;
            const inverseScale = 1 / this.scale;
            for (let index = 0; index < sources.surface.length; index++) {
                const vertex = sources.surface[index];
                const point = vertices[vertex];
                const offset = index * 3;
                positions[offset] = (point[0] - originX) * inverseScale;
                positions[offset + 1] = (point[1] - originY) * inverseScale;
                positions[offset + 2] = (point[2] - originZ) * inverseScale;
                writeNormal(vertex, restNormals, offset, normals);
            }
        }
        const gl = this.gl;
        gl.bindBuffer(gl.ARRAY_BUFFER, this.surfaces.buffers[0]);
        gl.bufferSubData(gl.ARRAY_BUFFER, 0, positions);
        gl.bindBuffer(gl.ARRAY_BUFFER, this.surfaces.buffers[1]);
        gl.bufferSubData(gl.ARRAY_BUFFER, 0, normals);
        this.posedVertices = vertices;
        this.poseEdgesDirty = true;
        this.dirty = true;
    }

    updatePoseEdges() {
        if (this.poseEdgesDirty === false) { return; }

        const { sources, edges } = this.poseBuffers;
        const [originX, originY, originZ] = this.origin;
        const inverseScale = 1 / this.scale;
        for (let index = 0; index < sources.edge.length; index++) {
            const point = this.posedVertices[sources.edge[index]];
            const offset = index * 3;
            edges[offset] = (point[0] - originX) * inverseScale;
            edges[offset + 1] = (point[1] - originY) * inverseScale;
            edges[offset + 2] = (point[2] - originZ) * inverseScale;
        }

        const gl = this.gl;
        gl.bindBuffer(gl.ARRAY_BUFFER, this.edges.buffers[0]);
        gl.bufferSubData(gl.ARRAY_BUFFER, 0, edges);
        this.poseEdgesDirty = false;
    }

    setRigPose(transforms, { bones, hierarchy = false, joints, mesh }) {
        this.showMesh = mesh;
        this.rigLineCount = 0;
        this.rigHierarchyCount = 0;
        this.rigJointCount = 0;
        this.dirty = true;
        if (this.contextLost === true || (bones === false && hierarchy === false && joints === false)) { return; }

        const skeleton = this.mesh?.rig?.bones ?? [];
        if (skeleton.length === 0) { return; }

        const origins = new Map(skeleton.map(bone => [bone.name, transforms?.get(bone.name)?.origin ?? bone.position]));
        const points = [];
        const append = point => {
            for (let axis = 0; axis < 3; axis++) { points.push((point[axis] - this.origin[axis]) / this.scale); }
        };
        if (bones === true) {
            for (const bone of skeleton) {
                if (bone.tail == null) { continue; }

                const [head, tail, ...ring] = boneDisplayPoints(bone, transforms?.get(bone.name));
                for (let index = 0; index < ring.length; index++) {
                    const shoulder = ring[index];
                    append(head);
                    append(shoulder);
                    append(shoulder);
                    append(tail);
                    append(shoulder);
                    append(ring[(index + 1) % ring.length]);
                }
            }
            this.rigLineCount = points.length / 3;
        }
        if (hierarchy === true) {
            for (const bone of skeleton) {
                if (bone.parent == null) { continue; }

                append(origins.get(bone.parent));
                append(origins.get(bone.name));
            }
            this.rigHierarchyCount = points.length / 3 - this.rigLineCount;
        }
        if (joints === true) {
            for (const bone of skeleton) { append(origins.get(bone.name)); }
            this.rigJointCount = skeleton.length;
        }

        const data = new Float32Array(points);
        if (this.rigGeometry?.count !== data.length / 3) {
            this.deleteGeometry(this.rigGeometry);
            this.rigGeometry = this.createGeometry(data);
        } else {
            this.gl.bindBuffer(this.gl.ARRAY_BUFFER, this.rigGeometry.buffers[0]);
            this.gl.bufferSubData(this.gl.ARRAY_BUFFER, 0, data);
        }
    }

    isObjectVisible(id) {
        return this.hiddenObjects.has(String(id)) === false;
    }

    setObjectVisible(id, visible) {
        const key = String(id);
        if (Object.hasOwn(this.mesh?.objects || {}, key) === false) { return; }

        if (visible === true) { this.hiddenObjects.delete(key); }
        else { this.hiddenObjects.add(key); }
        this.dirty = true;
    }

    showAllObjects() {
        this.hiddenObjects.clear();
        this.dirty = true;
    }

    isolateObjects(ids) {
        const visible = new Set(ids.map(String));
        this.hiddenObjects = new Set(Object.keys(this.mesh?.objects || {}).filter(id => visible.has(id) === false));
        this.dirty = true;
    }

    frame(object = null) {
        if (this.bounds === null) { return; }

        const aspect = Math.max(this.canvas.clientWidth / Math.max(this.canvas.clientHeight, 1), 0.1);
        const halfAngle = Math.min(Math.PI / 8, Math.atan(Math.tan(Math.PI / 8) * aspect));
        const vertices = Object.fromEntries(Object.entries(this.mesh.vertices).filter(([id]) => object === null ? this.isObjectVisible(this.vertexOwners.get(id)) : this.vertexOwners.get(id) === String(object)));
        if (Object.keys(vertices).length === 0) { return; }

        const bounds = measureMesh({ vertices });
        this.target = bounds.center.map((value, axis) => (value - this.origin[axis]) / this.scale);
        this.distance = bounds.radius / this.scale / Math.sin(halfAngle) * 1.3;
        this.dirty = true;
    }

    saveCamera() {
        const target = this.target.map((value, axis) => this.origin[axis] + value * this.scale);
        return { target, distance: this.distance * this.scale, yaw: this.yaw, pitch: this.pitch, orthographic: this.orthographic };
    }

    restoreCamera(camera) {
        const { target, distance, yaw, pitch } = camera ?? {};
        if (!Array.isArray(target) || target.length !== 3 || ![...target, distance, yaw, pitch].every(Number.isFinite)) { return; }

        const localTarget = target.map((value, axis) => (value - this.origin[axis]) / this.scale);
        const localDistance = distance / this.scale;
        if (!localTarget.every(Number.isFinite) || !Number.isFinite(localDistance) || localDistance <= 0) { return; }

        this.target = localTarget;
        this.distance = localDistance;
        this.yaw = yaw;
        this.pitch = Math.max(-Math.PI / 2, Math.min(Math.PI / 2, pitch));
        this.orthographic = camera.orthographic === true;
        this.dirty = true;
    }

    setCameraAngles(yaw, pitch) {
        this.yaw = yaw;
        this.pitch = pitch;
        this.dirty = true;
    }

    setOrthographic(enabled) {
        this.orthographic = enabled;
        this.dirty = true;
    }

    bindControls() {
        const canvas = this.canvas;
        const options = { signal: this.events.signal };
        canvas.addEventListener('contextmenu', event => event.preventDefault(), options);
        canvas.addEventListener('pointerdown', event => {
            event.preventDefault();
            canvas.focus();
            canvas.setPointerCapture(event.pointerId);
            this.pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
        }, options);
        canvas.addEventListener('pointermove', event => {
            const previous = this.pointers.get(event.pointerId);
            if (previous === undefined) { return; }

            const dx = event.clientX - previous.x;
            const dy = event.clientY - previous.y;
            const other = [...this.pointers.entries()].find(([id]) => id !== event.pointerId);
            if (other !== undefined) {
                const point = other[1];
                const oldDistance = Math.hypot(previous.x - point.x, previous.y - point.y);
                const newDistance = Math.hypot(event.clientX - point.x, event.clientY - point.y);
                if (newDistance > 1) { this.zoom(oldDistance / newDistance); }
                this.pan(dx / 2, dy / 2);
            } else if (event.buttons === 2 || event.buttons === 4 || event.shiftKey === true) {
                this.pan(dx, dy);
            } else {
                this.yaw -= dx * 0.006;
                this.pitch = Math.max(-1.5, Math.min(1.5, this.pitch + dy * 0.006));
            }
            this.pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
            this.dirty = true;
        }, options);
        const release = event => { this.pointers.delete(event.pointerId); };
        canvas.addEventListener('pointerup', release, options);
        canvas.addEventListener('pointercancel', release, options);
        canvas.addEventListener('lostpointercapture', release, options);
        canvas.addEventListener('wheel', event => {
            event.preventDefault();
            const units = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? canvas.clientHeight : 1;
            this.zoom(Math.exp(Math.max(-1, Math.min(1, event.deltaY * units * 0.001))));
        }, { ...options, passive: false });
        canvas.addEventListener('webglcontextlost', event => {
            event.preventDefault();
            this.contextLost = true;
            canvas.dispatchEvent(new CustomEvent('viewer-error', { detail: 'Graphics context lost. Waiting for the browser to restore it…' }));
        }, options);
        canvas.addEventListener('webglcontextrestored', () => {
            this.program = this.createProgram();
            for (const name of Object.keys(this.locations)) { this.locations[name] = this.gl.getUniformLocation(this.program, name); }
            this.surfaces = null;
            this.edges = null;
            this.ground = null;
            this.rigGeometry = null;
            this.contextLost = false;
            this.textures.clear();
            this.fallbackTexture = this.createTexture(1, 1, new Uint8Array([255, 255, 255, 255]));
            if (this.mesh !== null) { this.setMesh(this.mesh); }
            canvas.dispatchEvent(new CustomEvent('viewer-restored'));
        }, options);
    }

    zoom(factor) {
        const radius = this.bounds?.radius ?? 1;
        this.distance = Math.max(radius * 0.02, Math.min(radius * 100, this.distance * factor));
        this.dirty = true;
    }

    pan(dx, dy) {
        const amount = 2 * this.distance * Math.tan(Math.PI / 8) / Math.max(this.canvas.clientHeight, 1);
        const right = [Math.cos(this.yaw), 0, -Math.sin(this.yaw)];
        const up = [-Math.sin(this.pitch) * Math.sin(this.yaw), Math.cos(this.pitch), -Math.sin(this.pitch) * Math.cos(this.yaw)];
        this.target = this.target.map((value, axis) => value - right[axis] * dx * amount + up[axis] * dy * amount);
        this.dirty = true;
    }

    animate(time) {
        if (this.disposed === true) { return; }

        const elapsed = this.lastTime === null ? 0 : (time - this.lastTime) / 1000;
        this.lastTime = time;
        if (this.contextLost === false && document.hidden === false) {
            this.onAnimationFrame?.(elapsed);
        }
        if (this.turntable === true && this.pointers.size === 0 && document.hidden === false) {
            this.yaw += Math.min(elapsed, 0.1) * 0.22;
            this.dirty = true;
        }
        const ratio = Math.min(window.devicePixelRatio || 1, 2);
        const width = Math.max(1, Math.round(this.canvas.clientWidth * ratio));
        const height = Math.max(1, Math.round(this.canvas.clientHeight * ratio));
        if (this.canvas.width !== width || this.canvas.height !== height) {
            this.canvas.width = width;
            this.canvas.height = height;
            this.dirty = true;
        }
        if (this.dirty === true && this.contextLost === false && document.hidden === false) {
            this.render();
            this.dirty = false;
        }
        this.animation = requestAnimationFrame(this.animate);
    }

    render() {
        const gl = this.gl;
        const modes = { shaded: 0, topology: 0, normals: 1, depth: 2, silhouette: 3, orientation: 4, face_ids: 5, object_ids: 6, collection_ids: 7, uv_checker: 8, textured: 9, vertex_colors: 10, material: 11, vertex_color_shaded: 12 };
        const surfaceMode = modes[this.displayMode] ?? 0;
        const background = this.displayMode === 'depth' ? [0, 0, 0] : this.displayMode === 'silhouette' ? [1, 1, 1] : [0.090, 0.102, 0.125];
        gl.viewport(0, 0, this.canvas.width, this.canvas.height);
        gl.clearColor(...background, 1);
        gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
        if (this.bounds === null) { return; }

        const eye = [this.target[0] + Math.sin(this.yaw) * Math.cos(this.pitch) * this.distance, this.target[1] + Math.sin(this.pitch) * this.distance, this.target[2] + Math.cos(this.yaw) * Math.cos(this.pitch) * this.distance];
        const camera = cameraMatrix(eye, this.target, this.yaw);
        const centerDistance = Math.hypot(...eye.map((value, axis) => value - this.bounds.center[axis]));
        const near = Math.max(this.bounds.radius * 0.0001, centerDistance - this.bounds.radius * 1.5);
        const far = Math.max(near + 1, centerDistance + this.bounds.radius * 100);
        const aspect = this.canvas.width / this.canvas.height;
        const projection = this.orthographic ? orthographic(this.distance * Math.tan(Math.PI / 8), aspect, -far, far) : perspective(Math.PI / 4, aspect, near, far);
        gl.useProgram(this.program);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, this.fallbackTexture);
        gl.uniform1i(this.locations.textureImage, 0);
        gl.uniform1i(this.locations.hasTexture, false);
        gl.uniform1f(this.locations.alphaCutoff, -1);
        for (const name of ['hasNormal', 'hasRoughness', 'hasMetallic', 'hasSpecular']) { gl.uniform1i(this.locations[name], false); }
        gl.uniformMatrix4fv(this.locations.projection, false, projection);
        gl.uniformMatrix4fv(this.locations.camera, false, camera);
        gl.uniform3fv(this.locations.eye, eye);
        gl.uniform1i(this.locations.showGrid, this.grid);
        gl.uniform3fv(this.locations.groundCenter, this.bounds.center);
        gl.uniform1f(this.locations.groundRadius, this.bounds.radius);
        gl.uniform1i(this.locations.surfaceMode, surfaceMode);
        if (this.displayMode === 'depth') {
            const towardCamera = [Math.sin(this.yaw) * Math.cos(this.pitch), Math.sin(this.pitch), Math.cos(this.yaw) * Math.cos(this.pitch)];
            let minimumDepth = Infinity;
            let maximumDepth = -Infinity;
            for (const { id, object, point } of this.depthPoints) {
                if (this.isObjectVisible(object) === false) { continue; }

                const posed = this.posedVertices?.[id];
                const depth = posed === undefined
                    ? point.reduce((sum, value, axis) => sum + value * towardCamera[axis], 0)
                    : posed.reduce((sum, value, axis) => sum + (value - this.origin[axis]) * towardCamera[axis], 0) / this.scale;
                minimumDepth = Math.min(minimumDepth, depth);
                maximumDepth = Math.max(maximumDepth, depth);
            }

            gl.uniform3fv(this.locations.towardCamera, towardCamera);
            gl.uniform2fv(this.locations.depthRange, Number.isFinite(minimumDepth) ? [minimumDepth, maximumDepth] : [0, 0]);
        }
        gl.enable(gl.DEPTH_TEST);
        gl.disable(gl.CULL_FACE);
        const litSurface = surfaceMode === 0 || surfaceMode === 11 || surfaceMode === 12;
        if (litSurface === true && eye[1] > this.bounds.groundY) {
            const groundProjection = this.orthographic ? projection : perspective(Math.PI / 4, aspect, this.bounds.radius * 0.001, far);
            gl.uniformMatrix4fv(this.locations.projection, false, groundProjection);
            gl.depthMask(false);
            gl.uniform1i(this.locations.mode, 1);
            gl.bindVertexArray(this.ground.vertexArray);
            gl.drawArrays(gl.TRIANGLES, 0, this.ground.count);
            gl.depthMask(true);
            gl.uniformMatrix4fv(this.locations.projection, false, projection);
        }
        if (this.showMesh === true) {
            gl.uniform1i(this.locations.mode, 0);
            gl.enable(gl.POLYGON_OFFSET_FILL);
            gl.polygonOffset(1, 1);
            gl.bindVertexArray(this.surfaces.vertexArray);
            this.drawVisible(gl.TRIANGLES, false);
            gl.disable(gl.POLYGON_OFFSET_FILL);
            if (this.wireframe === true || this.displayMode === 'topology') {
                this.updatePoseEdges();
                gl.uniform1i(this.locations.mode, 2);
                gl.bindVertexArray(this.edges.vertexArray);
                this.drawVisible(gl.LINES, true);
            }
        }
        if (this.rigGeometry !== null && this.rigLineCount + this.rigHierarchyCount + this.rigJointCount > 0) {
            gl.disable(gl.DEPTH_TEST);
            gl.bindVertexArray(this.rigGeometry.vertexArray);
            if (this.rigHierarchyCount > 0) {
                gl.uniform1i(this.locations.mode, 5);
                gl.drawArrays(gl.LINES, this.rigLineCount, this.rigHierarchyCount);
            }
            if (this.rigLineCount > 0) {
                gl.uniform1i(this.locations.mode, 3);
                gl.drawArrays(gl.LINES, 0, this.rigLineCount);
            }
            if (this.rigJointCount > 0) {
                gl.uniform1i(this.locations.mode, 4);
                gl.drawArrays(gl.POINTS, this.rigLineCount + this.rigHierarchyCount, this.rigJointCount);
            }
        }
        gl.bindVertexArray(null);
    }

    drawVisible(mode, edges) {
        if (this.hiddenObjects.size === 0 && (edges === true || this.textures.size === 0)) {
            this.gl.drawArrays(mode, 0, edges === true ? this.edges.count : this.surfaces.count);
            return;
        }

        for (const group of this.groups) {
            if (this.isObjectVisible(group.object) === false) { continue; }
            if (edges === false) {
                const texture = this.textures.get(group.texture);
                this.gl.activeTexture(this.gl.TEXTURE0);
                this.gl.bindTexture(this.gl.TEXTURE_2D, texture?.handle || this.fallbackTexture);
                this.gl.uniform1i(this.locations.hasTexture, texture !== undefined);
                this.gl.uniform1f(this.locations.alphaCutoff, texture?.alphaCutoff ?? -1);
                for (const [index, channel] of ['normal', 'roughness', 'metallic', 'specular'].entries()) {
                    const handle = texture?.maps?.[channel];
                    const title = channel[0].toUpperCase() + channel.slice(1);
                    this.gl.activeTexture(this.gl.TEXTURE0 + index + 1);
                    this.gl.bindTexture(this.gl.TEXTURE_2D, handle || this.fallbackTexture);
                    this.gl.uniform1i(this.locations[channel + 'Image'], index + 1);
                    this.gl.uniform1i(this.locations['has' + title], handle !== undefined);
                }
                this.gl.activeTexture(this.gl.TEXTURE0);
            }
            this.gl.drawArrays(mode, edges === true ? group.edgeFirst : group.first, edges === true ? group.edgeCount : group.count);
        }
    }
}
