export function measureMesh(mesh) {
    const minimum = [Infinity, Infinity, Infinity];
    const maximum = [-Infinity, -Infinity, -Infinity];
    const vertices = Object.values(mesh.vertices);
    if (vertices.length === 0) { throw new Error('The mesh has no vertices.'); }

    for (const point of vertices) {
        for (let axis = 0; axis < 3; axis++) {
            if (Number.isFinite(point[axis]) === false) { throw new Error('The mesh contains invalid coordinates.'); }
            minimum[axis] = Math.min(minimum[axis], point[axis]);
            maximum[axis] = Math.max(maximum[axis], point[axis]);
        }
    }
    const center = minimum.map((value, axis) => value / 2 + maximum[axis] / 2);
    const extent = maximum.map((value, axis) => value - minimum[axis]);
    const radius = Math.max(Math.hypot(...extent) / 2, 0.000001);
    return { minimum, maximum, center, radius };
}

export function triangulateFace(points) {
    const fan = points.slice(2).map((_, index) => [0, index + 1, index + 2]);
    if (points.length <= 3) { return fan; }
    const normal = [0, 0, 0];
    const origin = points[0];
    const relative = points.map(point => point.map((value, axis) => value - origin[axis]));
    for (let index = 0; index < points.length; index++) {
        const a = relative[index];
        const b = relative[(index + 1) % points.length];
        for (let axis = 0; axis < 3; axis++) { normal[axis] += a[(axis + 1) % 3] * b[(axis + 2) % 3] - a[(axis + 2) % 3] * b[(axis + 1) % 3]; }
    }
    const axis = normal.map(Math.abs).indexOf(Math.max(...normal.map(Math.abs)));
    const flat = relative.map(point => [point[(axis + 1) % 3], point[(axis + 2) % 3]]);
    const cross = (a, b, c) => (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    const turns = flat.map((a, index) => cross(a, flat[(index + 1) % flat.length], flat[(index + 2) % flat.length]));
    if (turns.some(value => value > 0) === false || turns.some(value => value < 0) === false) { return fan; }
    if (points.length > 4096) { throw new Error('Concave faces support at most 4096 corners.'); }

    const area = flat.reduce((sum, a, index) => { const b = flat[(index + 1) % flat.length]; return sum + a[0] * b[1] - b[0] * a[1]; }, 0);
    const sign = area >= 0 ? 1 : -1;
    const scale = Math.max(...flat.flat().map(Math.abs));
    const epsilon = scale * scale * 1e-12;
    const remaining = flat.map((_, index) => index);
    const triangles = [];
    while (remaining.length > 3) {
        let clipped = false;
        for (let index = 0; index < remaining.length; index++) {
            const triangle = [remaining[(index + remaining.length - 1) % remaining.length], remaining[index], remaining[(index + 1) % remaining.length]];
            const [a, b, c] = triangle.map(vertex => flat[vertex]);
            if (cross(a, b, c) * sign <= epsilon) { continue; }
            const contains = remaining.some(vertex => triangle.includes(vertex) === false && cross(a, b, flat[vertex]) * sign >= -epsilon && cross(b, c, flat[vertex]) * sign >= -epsilon && cross(c, a, flat[vertex]) * sign >= -epsilon);
            if (contains === true) { continue; }
            triangles.push(triangle);
            remaining.splice(index, 1);
            clipped = true;
            break;
        }
        if (clipped === false) { throw new Error('Cannot triangulate a degenerate or crossing face.'); }
    }
    triangles.push(remaining);
    return triangles;
}

export function buildMeshData(mesh, origin, scale) {
    const poseSources = mesh.rig === undefined ? null : { surface: [], edge: [] };
    const positions = [];
    const normals = [];
    const faceColors = [];
    const objectColors = [];
    const collectionColors = [];
    const uvCoordinates = [];
    const vertexColors = [];
    const collectionIds = Object.keys(mesh.collections || {}).sort((a, b) => Number(a) - Number(b));
    const collectionPalette = facePalette(collectionIds.length);
    const colorsByCollection = new Map(collectionIds.map((id, index) => [id, collectionPalette[index]]));
    const groups = [];
    const owners = new Map();
    const objectIds = Object.keys(mesh.objects || {}).sort((a, b) => Number(a) - Number(b));
    const objectPalette = facePalette(Math.max(objectIds.length, 1));
    const colorsByObject = new Map();
    for (const [index, id] of objectIds.entries()) {
        colorsByObject.set(id, objectPalette[index]);
        for (const vertex of mesh.objects[id].vertices) { owners.set(String(vertex), id); }
    }
    const edges = [];
    const edgeKeys = new Set();
    const points = {};
    for (const [id, point] of Object.entries(mesh.vertices)) {
        points[id] = point.map((value, axis) => (value - origin[axis]) / scale);
    }
    const faceIds = Object.keys(mesh.faces);
    const faces = Object.values(mesh.faces);
    const cornerNormals = buildCornerNormals(mesh);
    const palette = facePalette(faces.length);
    for (const [faceIndex, face] of faces.entries()) {
        const object = owners.get(String(face[0])) ?? null;
        const faceId = faceIds[faceIndex];
        const texture = mesh.face_textures?.[faceId] ?? null;
        const uvValues = mesh.uvs?.[faceId] || {};
        const complete = face.every(vertex => Object.hasOwn(uvValues, vertex));
        let group = groups.at(-1);
        if (group === undefined || group.object !== object || group.texture !== texture) {
            group = { object, texture, first: positions.length / 3, count: 0, edgeFirst: edges.length / 3, edgeCount: 0 };
            groups.push(group);
        }
        const normal = [0, 0, 0];
        for (let index = 0; index < face.length; index++) {
            const firstId = face[index];
            const secondId = face[(index + 1) % face.length];
            const first = points[firstId];
            const second = points[secondId];
            if (first === undefined || second === undefined) { throw new Error('A face references a missing vertex.'); }

            normal[0] += (first[1] - second[1]) * (first[2] + second[2]);
            normal[1] += (first[2] - second[2]) * (first[0] + second[0]);
            normal[2] += (first[0] - second[0]) * (first[1] + second[1]);
            const key = firstId < secondId ? `${firstId}:${secondId}` : `${secondId}:${firstId}`;
            if (edgeKeys.has(key) === false) {
                edgeKeys.add(key);
                edges.push(...first, ...second);
                poseSources?.edge.push(firstId, secondId);
            }
        }
        const length = Math.hypot(...normal);
        if (length < 1e-15) { continue; }

        const triangles = triangulateFace(face.map(vertex => points[vertex]));
        for (const [first, second, third] of triangles) {
            for (const vertex of [face[first], face[second], face[third]]) {
                poseSources?.surface.push(String(vertex));
                const uv = uvValues[vertex] || [0, 0];
                uvCoordinates.push(...uv, complete === true ? 1 : 0);
                vertexColors.push(...(mesh.vertex_colors?.[vertex] || [1, 1, 1]));
            }
            positions.push(...points[face[first]], ...points[face[second]], ...points[face[third]]);
            normals.push(...cornerNormals[faceIndex][first], ...cornerNormals[faceIndex][second], ...cornerNormals[faceIndex][third]);
            const color = palette[faceIndex].map(value => value / 255);
            const objectColor = (colorsByObject.get(object) || objectPalette[0]).map(value => value / 255);
            faceColors.push(...color, ...color, ...color);
            objectColors.push(...objectColor, ...objectColor, ...objectColor);
            const collectionColor = (colorsByCollection.get(String(mesh.primary_collections?.[object])) || [128, 128, 128]).map(value => value / 255);
            collectionColors.push(...collectionColor, ...collectionColor, ...collectionColor);
        }
        group.count = positions.length / 3 - group.first;
        group.edgeCount = edges.length / 3 - group.edgeFirst;
    }
    return { positions: new Float32Array(positions), normals: new Float32Array(normals), faceColors: new Float32Array(faceColors), objectColors: new Float32Array(objectColors), collectionColors: new Float32Array(collectionColors), uvCoordinates: new Float32Array(uvCoordinates), vertexColors: new Float32Array(vertexColors), edges: new Float32Array(edges), groups, owners, poseSources };
}

export function buildCornerNormals(mesh) {
    const unit = value => { const length = Math.hypot(...value); return length > 0 ? value.map(n => n / length) : [0, 0, 0]; };
    const dot = (a, b) => a.reduce((sum, value, axis) => sum + value * b[axis], 0);
    const edgeKey = (a, b) => a < b ? `${a}:${b}` : `${b}:${a}`;
    const faces = Object.values(mesh.faces);
    const edges = new Map();
    const flatFaces = new Set(mesh.flat_faces || []);
    const faceIds = Object.keys(mesh.faces);
    const owners = new Map();
    for (const object of Object.values(mesh.objects || {})) {
        for (const vertex of object.vertices) { owners.set(vertex, object.smooth_angle); }
    }
    const normals = faces.map((face, index) => {
        const normal = [0, 0, 0];
        const origin = mesh.vertices[face[0]];
        for (let i = 0; i < face.length; i++) {
            const a = mesh.vertices[face[i]];
            const b = mesh.vertices[face[(i + 1) % face.length]];
            for (let axis = 0; axis < 3; axis++) {
                const j = (axis + 1) % 3;
                const k = (axis + 2) % 3;
                normal[axis] += (a[j] - origin[j]) * (b[k] - origin[k]) - (a[k] - origin[k]) * (b[j] - origin[j]);
            }
            const key = edgeKey(face[i], face[(i + 1) % face.length]);
            if (edges.has(key) === false) { edges.set(key, []); }
            edges.get(key).push(index);
        }
        return unit(normal);
    });
    return faces.map((face, index) => face.map(vertex => {
        if (flatFaces.has(Number(faceIds[index]))) { return normals[index]; }
        const angle = owners.get(vertex);
        if (angle === undefined || angle === null || angle === 0) { return normals[index]; }
        const cosine = Math.cos(angle * Math.PI / 180);
        const pending = [index];
        const seen = new Set();
        const sum = [0, 0, 0];
        while (pending.length > 0) {
            const current = pending.pop();
            if (seen.has(current)) { continue; }
            seen.add(current);
            const polygon = faces[current];
            const i = polygon.indexOf(vertex);
            const before = polygon[(i + polygon.length - 1) % polygon.length];
            const after = polygon[(i + 1) % polygon.length];
            const a = unit(mesh.vertices[before].map((n, axis) => n - mesh.vertices[vertex][axis]));
            const b = unit(mesh.vertices[after].map((n, axis) => n - mesh.vertices[vertex][axis]));
            const weight = Math.acos(Math.max(-1, Math.min(1, dot(a, b))));
            for (let axis = 0; axis < 3; axis++) { sum[axis] += normals[current][axis] * weight; }
            for (const neighbor of [before, after]) {
                for (const adjacent of edges.get(edgeKey(vertex, neighbor))) {
                    if (dot(normals[current], normals[adjacent]) >= cosine - 1e-10) { pending.push(adjacent); }
                }
            }
        }
        const normal = unit(sum);
        return normal.every(value => value === 0) ? normals[index] : normal;
    }));
}

export function facePalette(count) {
    if (count > 0xffffff) { throw new Error('Too many faces for unique 24-bit face colors.'); }

    const used = new Set();
    const colors = [];
    for (let index = 0; index < count; index++) {
        const hue = (index * 0.618033988749895 % 1) * 6;
        const sector = Math.floor(hue);
        const fraction = hue - sector;
        const value = 0.95;
        const p = value * (1 - 0.78);
        const q = value * (1 - 0.78 * fraction);
        const t = value * (1 - 0.78 * (1 - fraction));
        const rgb = [[value,t,p], [q,value,p], [p,value,t], [p,q,value], [t,p,value], [value,p,q]][sector];
        const bytes = rgb.map(channel => Math.round(channel * 255));
        let code = (bytes[0] << 16) | (bytes[1] << 8) | bytes[2];
        if (used.has(code) === true) { code = Math.imul(index + 1, 0x9e3779) & 0xffffff; }
        while (code === 0 || used.has(code) === true) { code = (code + 1) & 0xffffff; }
        used.add(code);
        colors.push([code >> 16, code >> 8 & 255, code & 255]);
    }
    return colors;
}

export function perspective(fieldOfView, aspect, near, far) {
    const focal = 1 / Math.tan(fieldOfView / 2);
    const range = 1 / (near - far);
    return new Float32Array([focal / aspect, 0, 0, 0, 0, focal, 0, 0, 0, 0, (far + near) * range, -1, 0, 0, 2 * far * near * range, 0]);
}

export function orthographic(halfHeight, aspect, near, far) {
    const range = 1 / (near - far);
    return new Float32Array([1 / (halfHeight * aspect), 0, 0, 0, 0, 1 / halfHeight, 0, 0, 0, 0, 2 * range, 0, 0, 0, (far + near) * range, 1]);
}

export function cameraMatrix(eye, target, yaw = 0) {
    const backward = eye.map((value, axis) => value - target[axis]);
    const length = Math.hypot(...backward);
    const z = backward.map(value => value / length);
    const rightLength = Math.hypot(z[2], z[0]);
    const x = rightLength > 1e-10 ? [z[2] / rightLength, 0, -z[0] / rightLength] : [Math.cos(yaw), 0, -Math.sin(yaw)];
    const y = [z[1] * x[2], z[2] * x[0] - z[0] * x[2], -z[1] * x[0]];
    const offset = direction => -direction.reduce((sum, value, axis) => sum + value * eye[axis], 0);
    return new Float32Array([x[0], y[0], z[0], 0, x[1], y[1], z[1], 0, x[2], y[2], z[2], 0, offset(x), offset(y), offset(z), 1]);
}
