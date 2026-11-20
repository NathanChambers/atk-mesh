const identity = [0, 0, 0, 1];
const zero = [0, 0, 0];
const add = (a, b) => a.map((value, axis) => value + b[axis]);
const subtract = (a, b) => a.map((value, axis) => value - b[axis]);
const scale = (point, factor) => point.map(value => value * factor);
const cross = ([x, y, z], [a, b, c]) => [y * c - z * b, z * a - x * c, x * b - y * a];

export function boneDisplayPoints(bone, pose) {
    const offset = subtract(bone.tail, bone.position);
    const length = Math.hypot(...offset);
    const direction = scale(offset, 1 / length);
    const reference = Math.abs(direction[2]) < 0.9 ? [0, 0, 1] : [0, 1, 0];
    const across = cross(reference, direction);
    const u = scale(across, length * 0.095 / Math.hypot(...across));
    const v = cross(direction, u);
    const shoulder = scale(offset, 0.24);
    const points = [zero, offset, add(shoulder, u), add(shoulder, v), subtract(shoulder, u), subtract(shoulder, v)];
    const { origin, rotation } = pose ?? { origin: bone.position, rotation: identity };
    return points.map(point => add(origin, rotate(rotation, point)));
}

function multiply([ax, ay, az, aw], [bx, by, bz, bw]) {
    return [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ];
}

function rotate(rotation, [x, y, z]) {
    const [qx, qy, qz, qw] = rotation;
    const tx = 2 * (qy * z - qz * y);
    const ty = 2 * (qz * x - qx * z);
    const tz = 2 * (qx * y - qy * x);
    return [x + qw * tx + qy * tz - qz * ty, y + qw * ty + qz * tx - qx * tz, z + qw * tz + qx * ty - qy * tx];
}

function writeRotationMatrix([x, y, z, w], matrix) {
    const xx = 2 * x * x;
    const yy = 2 * y * y;
    const zz = 2 * z * z;
    const xy = 2 * x * y;
    const xz = 2 * x * z;
    const yz = 2 * y * z;
    const wx = 2 * w * x;
    const wy = 2 * w * y;
    const wz = 2 * w * z;

    matrix[0] = 1 - yy - zz;
    matrix[1] = xy - wz;
    matrix[2] = xz + wy;
    matrix[3] = xy + wz;
    matrix[4] = 1 - xx - zz;
    matrix[5] = yz - wx;
    matrix[6] = xz - wy;
    matrix[7] = yz + wx;
    matrix[8] = 1 - xx - yy;
}

function quaternion(degrees) {
    const [x, y, z] = degrees.map(value => value % 360 * Math.PI / 360);
    const aroundX = [Math.sin(x), 0, 0, Math.cos(x)];
    const aroundY = [0, Math.sin(y), 0, Math.cos(y)];
    const aroundZ = [0, 0, Math.sin(z), Math.cos(z)];
    return multiply(aroundZ, multiply(aroundY, aroundX));
}

function interpolateRotation(start, end, fraction) {
    const dot = start.reduce((sum, value, axis) => sum + value * end[axis], 0);
    const sign = dot < 0 ? -1 : 1;
    if (Math.abs(dot) > 0.9995) {
        const blended = start.map((value, axis) => value + (sign * end[axis] - value) * fraction);
        const length = Math.hypot(...blended);
        return blended.map(value => value / length);
    }

    const angle = Math.acos(Math.abs(dot));
    const a = Math.sin((1 - fraction) * angle) / Math.sin(angle);
    const b = sign * Math.sin(fraction * angle) / Math.sin(angle);
    return start.map((value, axis) => a * value + b * end[axis]);
}

function sample(keys, time, interpolation) {
    if (keys === undefined) { return { rotation: identity, translation: zero }; }
    const index = keys.findIndex(key => key.time > time);
    if (index === 0) { return keys[0]; }
    if (index === -1) { return keys.at(-1); }

    const start = keys[index - 1];
    const end = keys[index];
    if (interpolation === 'step') { return start; }

    let fraction = (time - start.time) / (end.time - start.time);
    if (interpolation === 'cubic') {
        fraction *= fraction * (3 - 2 * fraction);
    }
    return {
        rotation: interpolateRotation(start.rotation, end.rotation, fraction),
        translation: start.translation.map((value, axis) => value + (end.translation[axis] - value) * fraction),
    };
}

export function createAnimationSampler(mesh, clip) {
    const bones = new Map();
    for (const bone of mesh.rig.bones) {
        const parent = bones.get(bone.parent);
        const offset = parent === undefined ? bone.position : subtract(bone.position, parent.position);
        const keys = Object.hasOwn(clip.tracks, bone.name) ? clip.tracks[bone.name] : undefined;
        const track = keys?.map(key => ({ ...key, rotation: quaternion(key.rotation) }));
        const interpolation = clip.interpolation?.[bone.name];
        bones.set(bone.name, { position: bone.position, parent, offset, track, interpolation, matrix: new Float64Array(9) });
    }
    const groups = new Map();
    const bindings = new Map();
    const playbackVertices = { ...mesh.vertices };
    const bound = Object.entries(mesh.rig.weights).map(([id, influences]) => {
        const key = JSON.stringify(influences);
        let group = groups.get(key);
        if (group === undefined) {
            group = {
                offset: groups.size * 12,
                influences: influences.map(({ bone, weight }) => ({ joint: bones.get(bone), weight })),
            };
            groups.set(key, group);
        }
        bindings.set(id, group.offset);
        const output = [0, 0, 0];
        playbackVertices[id] = output;
        return { id, point: mesh.vertices[id], output, offset: group.offset };
    });
    const transforms = new Float64Array(groups.size * 12);
    let surfaceSources = null;
    let surfacePoints = null;
    let surfaceOffsets = null;

    const samplePose = (time, reusePlaybackBuffer = false) => {
        for (const joint of bones.values()) {
            const { rotation, translation } = sample(joint.track, time, joint.interpolation);
            const parentRotation = joint.parent?.rotation ?? identity;
            const parentPosition = joint.parent?.origin ?? zero;
            joint.rotation = multiply(parentRotation, rotation);
            joint.origin = add(parentPosition, rotate(parentRotation, add(joint.offset, translation)));
            writeRotationMatrix(joint.rotation, joint.matrix);
            const [x, y, z] = joint.position;
            const m = joint.matrix;
            joint.tx = joint.origin[0] - (m[0] * x + m[1] * y + m[2] * z);
            joint.ty = joint.origin[1] - (m[3] * x + m[4] * y + m[5] * z);
            joint.tz = joint.origin[2] - (m[6] * x + m[7] * y + m[8] * z);
        }

        transforms.fill(0);
        for (const { offset, influences } of groups.values()) {
            for (const { joint, weight } of influences) {
                for (let i = 0; i < 9; i++) { transforms[offset + i] += weight * joint.matrix[i]; }
                transforms[offset + 9] += weight * joint.tx;
                transforms[offset + 10] += weight * joint.ty;
                transforms[offset + 11] += weight * joint.tz;
            }
        }
        const vertices = reusePlaybackBuffer === true ? playbackVertices : { ...mesh.vertices };
        for (const { id, point: [x, y, z], output, offset } of bound) {
            const point = reusePlaybackBuffer === true ? output : [0, 0, 0];
            point[0] = transforms[offset] * x + transforms[offset + 1] * y + transforms[offset + 2] * z + transforms[offset + 9];
            point[1] = transforms[offset + 3] * x + transforms[offset + 4] * y + transforms[offset + 5] * z + transforms[offset + 10];
            point[2] = transforms[offset + 6] * x + transforms[offset + 7] * y + transforms[offset + 8] * z + transforms[offset + 11];
            if (reusePlaybackBuffer === false) { vertices[id] = point; }
        }
        return vertices;
    };

    const writeTransformedNormal = (offset, source, index, target) => {
        const x = source[index];
        const y = source[index + 1];
        const z = source[index + 2];
        if (offset < 0) {
            target[index] = x;
            target[index + 1] = y;
            target[index + 2] = z;
            return;
        }

        const nx = transforms[offset] * x + transforms[offset + 1] * y + transforms[offset + 2] * z;
        const ny = transforms[offset + 3] * x + transforms[offset + 4] * y + transforms[offset + 5] * z;
        const nz = transforms[offset + 6] * x + transforms[offset + 7] * y + transforms[offset + 8] * z;
        const length = Math.sqrt(nx * nx + ny * ny + nz * nz);
        target[index] = length > 0 ? nx / length : x;
        target[index + 1] = length > 0 ? ny / length : y;
        target[index + 2] = length > 0 ? nz / length : z;
    };
    const writeNormal = (vertex, source, index, target) => {
        writeTransformedNormal(bindings.get(vertex) ?? -1, source, index, target);
    };
    const writeSurface = (sources, restNormals, positions, normals, origin, scale) => {
        if (surfaceSources !== sources) {
            surfacePoints = sources.map(vertex => playbackVertices[vertex]);
            surfaceOffsets = Int32Array.from(sources, vertex => bindings.get(vertex) ?? -1);
            surfaceSources = sources;
        }
        const [x, y, z] = origin;
        const inverseScale = 1 / scale;
        for (let i = 0; i < surfacePoints.length; i++) {
            const point = surfacePoints[i];
            const offset = 3 * i;
            positions[offset] = (point[0] - x) * inverseScale;
            positions[offset + 1] = (point[1] - y) * inverseScale;
            positions[offset + 2] = (point[2] - z) * inverseScale;
            writeTransformedNormal(surfaceOffsets[i], restNormals, offset, normals);
        }
    };
    const boneTransforms = () => new Map([...bones].map(([name, { origin, rotation }]) => [name, { origin, rotation }]));
    return { samplePose, writeNormal, writeSurface, boneTransforms };
}
