import { ModelViewer } from './renderer.js';
import { SessionReplay } from './replay.js';
import { ObjectInspector } from './objects.js';
import { createAnimationSampler } from './animation.js';

const element = id => document.getElementById(id);
const workspaceTarget = new URLSearchParams(window.location.search).get('workspace');
const workspaceQuery = workspaceTarget === null ? '' : `?${new URLSearchParams({ workspace: workspaceTarget })}`;
let viewer = null;
let geometryKey = null;
let savedStateTag = null;
let timer = null;
let stopped = false;
let controller = null;
let graphicsError = null;
let replay = null;
let inspector = null;
let workspace = null;
let savedViewerSnapshot = null;
let workspaceMode = null;
let workspacePaths = [];
let workspaceMatches = [];
let workspaceSelection = -1;
let nextDiscovery = 0;
let animation = null;
let clipMatches = [];
let clipHighlight = -1;
let viewerMode = 'mesh';
const renderModes = { mesh: 'shaded', animation: 'vertex_color_shaded' };
const rigDisplay = { bones: false, hierarchy: false, joints: false, mesh: true };
function setupCameraGizmo() {
    const directions = [
        ['right', 'axis-x', [1, 0, 0], Math.PI / 2, 0],
        ['left', 'axis-x', [-1, 0, 0], -Math.PI / 2, 0],
        ['top', 'axis-y', [0, 1, 0], 0, Math.PI / 2],
        ['bottom', 'axis-y', [0, -1, 0], 0, -Math.PI / 2],
        ['front', 'axis-z', [0, 0, 1], 0, 0],
        ['back', 'axis-z', [0, 0, -1], Math.PI, 0],
    ];
    const axes = directions.map(([name, color, direction, yaw, pitch]) => {
        const button = element(`camera-${name}`);
        const line = document.createElement('span');
        button.addEventListener('click', () => {
            viewer.setCameraAngles(yaw, pitch);
            viewer.turntable = false;
            element('turntable').setAttribute('aria-pressed', 'false');
            updateAxes();
        });
        return { button, line, color, direction };
    });
    element('camera-axis-lines').replaceChildren(...axes.map(axis => axis.line));

    let lastYaw = null;
    let lastPitch = null;
    function updateAxes() {
        const { yaw, pitch } = viewer;
        if (yaw === lastYaw && pitch === lastPitch) { return; }

        lastYaw = yaw;
        lastPitch = pitch;
        const right = [Math.cos(yaw), 0, -Math.sin(yaw)];
        const up = [-Math.sin(yaw) * Math.sin(pitch), Math.cos(pitch), -Math.cos(yaw) * Math.sin(pitch)];
        const facing = [Math.sin(yaw) * Math.cos(pitch), Math.sin(pitch), Math.cos(yaw) * Math.cos(pitch)];
        for (const { button, line, color, direction } of axes) {
            const project = basis => basis.reduce((sum, value, index) => sum + value * direction[index], 0);
            const depth = project(facing);
            let x = 40 * project(right);
            const y = -40 * project(up);
            // Separate coincident endpoints when an axis points toward the camera.
            x += Math.sign(depth) * Math.max(0, 14 - Math.hypot(x, y));
            const rear = depth < -0.0001 ? ' rear' : '';
            button.className = `camera-axis ${color}${rear}`;
            line.className = `camera-axis-line ${color}${rear}`;
            button.setAttribute('style', `transform: translate(calc(-50% + ${x}px), calc(-50% + ${y}px)); z-index: ${Math.round((depth + 1) * 10) + 1}`);
            button.setAttribute('aria-pressed', String(depth >= 1 - 1e-8));
            line.setAttribute('style', `width: ${Math.hypot(x, y)}px; transform: rotate(${Math.atan2(y, x)}rad)`);
        }
    }

    element('orthographic').addEventListener('click', () => {
        viewer.setOrthographic(!viewer.orthographic);
        syncProjectionButton();
    });
    updateAxes();
    return updateAxes;
}

function syncProjectionButton() {
    const button = element('orthographic');
    const { orthographic } = viewer;
    button.textContent = orthographic ? 'Orthographic' : 'Perspective';
    button.title = orthographic ? 'Switch to perspective projection' : 'Switch to orthographic projection';
    button.setAttribute('aria-pressed', String(orthographic));
}

function viewerSnapshot() {
    return JSON.stringify({
        camera: viewer.saveCamera(),
        grid: viewer.grid,
        turntable: viewer.turntable,
        viewerMode,
        renderModes,
        hiddenObjects: [...viewer.hiddenObjects].sort(),
        objectsOpen: element('objects-panel').hidden === false,
        expandedCollections: [...inspector.expandedCollections].sort(),
        rigDisplay,
        clip: animation === null ? null : { name: animation.name, time: animation.time },
    });
}

function saveWorkspaceView() {
    if (workspace === null || savedViewerSnapshot === null || replay.active === true) { return; }

    const snapshot = viewerSnapshot();
    if (snapshot === savedViewerSnapshot) { return; }

    try {
        window.localStorage.setItem(`gpt-mesh:workspace-view:1:${workspace}`, snapshot);
        savedViewerSnapshot = snapshot;
    } catch {
        // Viewing remains available if the browser disables storage or its quota is full.
    }
}

function restoreWorkspaceView() {
    let state;
    try {
        state = JSON.parse(window.localStorage.getItem(`gpt-mesh:workspace-view:1:${workspace}`));
    } catch {
        return;
    }
    if (state === null || typeof state !== 'object') { return; }

    viewer.restoreCamera(state.camera);
    syncProjectionButton();
    for (const name of ['grid', 'turntable']) {
        if (typeof state[name] !== 'boolean') { continue; }

        viewer[name] = state[name];
        element(name).setAttribute('aria-pressed', String(viewer[name]));
    }
    const modes = new Set([...document.querySelectorAll('[data-render-mode]')].map(button => button.dataset.renderMode));
    for (const name of Object.keys(renderModes)) {
        const mode = state.renderModes?.[name];
        if (modes.has(mode)) { renderModes[name] = mode; }
    }
    for (const name of Object.keys(rigDisplay)) {
        const enabled = state.rigDisplay?.[name];
        if (typeof enabled !== 'boolean') { continue; }

        rigDisplay[name] = enabled;
        element(`animation-${name}`).setAttribute('aria-pressed', String(enabled));
    }
    if (Array.isArray(state.hiddenObjects)) {
        for (const id of state.hiddenObjects) {
            if (typeof id === 'string') { viewer.setObjectVisible(id, false); }
        }
    }
    if (Array.isArray(state.expandedCollections)) {
        inspector.expandedCollections = new Set(state.expandedCollections.filter(id => typeof id === 'string'));
        inspector.setMesh(viewer.mesh);
    } else {
        inspector.updateVisibility();
    }
    if (state.objectsOpen === true) { inspector.setOpen(true); }
    if (state.viewerMode === 'animation') { selectViewerMode('animation'); }
    else { selectRenderMode(renderModes[viewerMode]); }
    if (typeof state.clip?.name === 'string' && Number.isFinite(state.clip.time)) {
        selectAnimation(state.clip.name, { time: Math.max(0, state.clip.time), playing: false });
    }
}

function showAnimationFrame() {
    if (viewerMode !== 'animation' || viewer.contextLost === true) { return; }

    let transforms = null;
    if (animation !== null) {
        const { samplePose, writeNormal, writeSurface, boneTransforms } = animation.sampler;
        viewer.setPose(samplePose(animation.time, true), writeNormal, writeSurface);
        if (rigDisplay.bones === true || rigDisplay.hierarchy === true || rigDisplay.joints === true) { transforms = boneTransforms(); }

        const time = element('animation-time');
        time.value = animation.time;
        time.setAttribute('aria-valuetext', `${animation.time.toFixed(2)} seconds`);
    }
    viewer.setRigPose(transforms, rigDisplay);
}

function selectAnimation(name, previous = null) {
    const clip = Object.hasOwn(viewer.mesh.rig?.clips || {}, name) ? viewer.mesh.rig.clips[name] : null;
    animation = clip === null ? null : {
        name, clip, sampler: createAnimationSampler(viewer.mesh, clip),
        time: Math.min(previous?.time ?? 0, clip.duration),
        playing: previous?.playing ?? viewerMode === 'animation',
    };
    element('animation-play').disabled = animation === null;
    element('animation-play').textContent = animation?.playing === true ? 'Pause' : 'Play';
    const clipName = element('animation-clip-name');
    clipName.textContent = animation?.name ?? 'Rest pose';
    clipName.title = clipName.textContent;
    const time = element('animation-time');
    time.disabled = animation === null;
    time.max = clip?.duration ?? 1;
    time.value = animation?.time ?? 0;
    if (animation === null) {
        time.removeAttribute('aria-valuetext');
        if (viewer.contextLost === false) { viewer.setMesh(viewer.mesh); }
    }
    showAnimationFrame();
}

function syncAnimationControls(mesh) {
    const bones = mesh.rig?.bones ?? [];
    element('animation-bones').disabled = !bones.some(bone => bone.tail != null);
    element('animation-hierarchy').disabled = !bones.some(bone => bone.parent != null);
    element('animation-joints').disabled = bones.length === 0;

    if (animation !== null) { selectAnimation(animation.name, animation); }
    else { showAnimationFrame(); }
    const clips = mesh.rig?.clips ?? {};
    element('animation-clip-open').disabled = Object.keys(clips).length === 0;
    if (element('animation-clip-picker').open === true) { renderClipMatches(); }
}

function renderClipMatches() {
    const clips = viewer.mesh?.rig?.clips ?? {};
    const search = element('animation-clip-search');
    const terms = search.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
    clipMatches = [['', 'Rest pose'], ...Object.keys(clips).map(name => [name, name])]
        .map(([name, label]) => ({ name, label, score: fuzzySearchScore(label, terms) }))
        .filter(match => Number.isFinite(match.score))
        .sort((left, right) => left.score - right.score);
    const selected = animation?.name ?? '';
    const options = clipMatches.map(({ name, label }, index) => {
        const option = document.createElement('div');
        option.id = `animation-clip-option-${index}`;
        option.setAttribute('role', 'option');
        option.setAttribute('aria-selected', String(name === selected));
        option.title = label;
        const title = document.createElement('span');
        title.textContent = label;
        if (name !== '') {
            const duration = document.createElement('small');
            duration.textContent = `${Number(clips[name].duration.toPrecision(4))} s`;
            option.replaceChildren(title, duration);
        } else {
            option.replaceChildren(title);
        }
        option.addEventListener('mousedown', event => event.preventDefault());
        option.addEventListener('click', () => chooseClip(name));
        return option;
    });
    element('animation-clip-results').replaceChildren(...options);
    element('animation-empty').hidden = options.length > 0;
    const current = terms.length === 0 ? clipMatches.findIndex(match => match.name === selected) : -1;
    highlightClip(Math.max(0, current));
}

function highlightClip(index) {
    const search = element('animation-clip-search');
    const options = element('animation-clip-results').children;
    clipHighlight = options.length === 0 ? -1 : index;
    if (clipHighlight === -1) {
        search.removeAttribute('aria-activedescendant');
        return;
    }

    for (const [position, option] of Array.from(options).entries()) {
        if (position === index) { option.setAttribute('data-highlighted', ''); }
        else { option.removeAttribute('data-highlighted'); }
    }
    const active = options[index];
    search.setAttribute('aria-activedescendant', active.id);
    if (document.activeElement === search) { active.scrollIntoView({ block: 'nearest' }); }
}

function chooseClip(name) {
    if (viewer.mesh === null) { return; }

    selectAnimation(name);
    element('animation-clip-picker').close();
}

function seekAnimation(time) {
    if (animation === null) { return; }

    animation.time = Math.max(0, Math.min(animation.clip.duration, time));
    animation.playing = false;
    element('animation-play').textContent = 'Play';
    showAnimationFrame();
}

function advanceAnimation(seconds) {
    if (viewerMode !== 'animation' || animation?.playing !== true || seconds <= 0) { return; }

    animation.time += seconds;
    const { duration, looping } = animation.clip;
    if (animation.time >= duration) {
        animation.time = looping ? animation.time % duration : duration;
        if (looping === false) {
            animation.playing = false;
            element('animation-play').textContent = 'Play';
        }
    }
    showAnimationFrame();
}

function selectRenderMode(mode) {
    renderModes[viewerMode] = mode;
    viewer.displayMode = mode;
    viewer.dirty = true;
    const debug = element('debug-views');
    let debugActive = false;
    for (const button of document.querySelectorAll('[data-render-mode]')) {
        const active = button.dataset.renderMode === mode;
        button.setAttribute('aria-pressed', String(active));
        if (active === true && debug.contains(button)) { debugActive = true; }
    }
    debug.toggleAttribute('data-active', debugActive);
    debug.querySelector('summary').setAttribute('aria-label', debugActive ? 'Debug (active)' : 'Debug');
}

function selectViewerMode(mode) {
    if (mode === viewerMode) { return; }

    if (viewerMode === 'animation' && animation !== null) {
        animation.playing = false;
        element('animation-play').textContent = 'Play';
        if (viewer.contextLost === false) { viewer.setMesh(viewer.mesh); }
    }

    viewerMode = mode;
    if (mode === 'mesh') { viewer.setRigPose(null, { bones: false, joints: false, mesh: true }); }
    for (const button of document.querySelectorAll('[data-viewer-mode]')) {
        button.setAttribute('aria-pressed', String(button.dataset.viewerMode === mode));
    }
    element('animation-controls').hidden = mode !== 'animation';
    selectRenderMode(renderModes[mode]);
    showAnimationFrame();
}

function setConnection(connected) {
    element('connection').textContent = connected === true ? 'Live' : 'Reconnecting';
    element('connection-dot').className = connected === true ? 'live' : 'offline';
}

function showState(state) {
    const nextGeometryKey = JSON.stringify([state.mesh.vertices, state.mesh.faces, state.mesh.objects, state.mesh.flat_faces, state.mesh.collections, state.mesh.primary_collections, state.mesh.uvs, state.mesh.textures, state.mesh.face_textures, state.mesh.vertex_colors, state.mesh.rig]);
    if (nextGeometryKey !== geometryKey) {
        viewer.setMesh(state.mesh);
        inspector.setMesh(state.mesh);
        syncAnimationControls(state.mesh);
        geometryKey = nextGeometryKey;
    }
    const revision = state.revision;
    element('revision').textContent = revision === null ? '' : `rev ${revision}`;
    const vertexCount = Object.keys(state.mesh.vertices).length;
    const faceCount = Object.keys(state.mesh.faces).length;
    const objectCount = Object.keys(state.mesh.objects || {}).length;
    element('statistics').textContent = `${objectCount} objects · ${vertexCount.toLocaleString()} vertices · ${faceCount.toLocaleString()} faces`;
}

async function readJson(path, options) {
    const response = await fetch(path, options);
    return checkedJson(response);
}

async function checkedJson(response) {
    const data = await response.json();
    if (data.ok !== true) { throw new Error(data.error || 'Unable to read the model.'); }
    return data;
}

async function discoverWorkspaces(options) {
    const data = await readJson('/workspaces', options);
    workspaceMode = data.result.mode;
    const projectViewer = workspaceMode === 'viewer';
    replay.workspaceQuery = projectViewer ? workspaceQuery : '';
    element('workspace-open').hidden = !projectViewer;
    if (projectViewer === false) { return; }

    const paths = data.result.workspaces.map(workspace => workspace.path);
    element('workspace-path').textContent = workspaceTarget ?? 'No workspace selected';
    element('workspace-path').title = workspaceTarget ?? '';
    if (paths.length === workspacePaths.length && paths.every((path, index) => path === workspacePaths[index])) { return; }

    workspacePaths = paths;
    if (element('workspace-picker').open === true) { renderWorkspaceMatches(); }
}

function fuzzySearchScore(label, terms) {
    const name = label.toLowerCase();
    let score = 0;
    for (const term of terms) {
        const exact = name.indexOf(term);
        if (exact !== -1) {
            score += exact / (name.length + 1);
            continue;
        }

        let cursor = 0;
        for (const character of term) {
            const position = name.indexOf(character, cursor);
            if (position === -1) { return Infinity; }
            cursor = position + 1;
        }
        score += 1 + cursor / (name.length + 1);
    }
    return score;
}

function renderWorkspaceMatches() {
    const search = element('workspace-search');
    const terms = search.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
    workspaceMatches = workspacePaths.map(path => ({ path, score: fuzzySearchScore(path, terms) }))
        .filter(match => Number.isFinite(match.score))
        .sort((left, right) => left.score - right.score)
        .map(match => match.path);
    const options = workspaceMatches.map((path, index) => {
        const option = document.createElement('div');
        option.id = `workspace-option-${index}`;
        option.setAttribute('role', 'option');
        option.textContent = path;
        option.addEventListener('click', () => selectWorkspace(path));
        return option;
    });
    element('workspace-results').replaceChildren(...options);
    element('workspace-empty').hidden = options.length > 0;
    const current = terms.length === 0 ? workspaceMatches.indexOf(workspaceTarget) : -1;
    highlightWorkspace(Math.max(0, current));
}

function highlightWorkspace(index) {
    const search = element('workspace-search');
    const options = element('workspace-results').children;
    workspaceSelection = options.length === 0 ? -1 : index;
    if (workspaceSelection === -1) {
        search.removeAttribute('aria-activedescendant');
        return;
    }

    for (const [position, option] of Array.from(options).entries()) {
        option.setAttribute('aria-selected', String(position === index));
    }
    search.setAttribute('aria-activedescendant', options[index].id);
    options[index].scrollIntoView({ block: 'nearest' });
}

function openWorkspacePicker() {
    const picker = element('workspace-picker');
    if (picker.open === true) {
        element('workspace-search').focus();
        return;
    }

    element('workspace-search').value = '';
    picker.showModal();
    renderWorkspaceMatches();
    nextDiscovery = 0;
}

function selectWorkspace(path) {
    if (path === workspaceTarget) {
        element('workspace-picker').close();
        return;
    }

    saveWorkspaceView();
    const url = new URL(window.location.href);
    url.searchParams.set('workspace', path);
    window.location.assign(url);
}

async function refresh() {
    if (stopped === true) { return; }

    controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 5000);
    try {
        const options = { signal: controller.signal, cache: 'no-store' };
        if (workspaceMode === null || (element('workspace-picker').open === true && Date.now() >= nextDiscovery)) {
            await discoverWorkspaces(options);
            nextDiscovery = Date.now() + 5000;
        }
        if (stopped === true) { return; }

        if (workspaceMode === 'viewer' && workspaceTarget === null) {
            element('notice').textContent = graphicsError ?? 'Choose a workspace with the folder button or Ctrl+P.';
            element('notice').hidden = false;
            setConnection(true);
            return;
        }

        const query = workspaceMode === 'viewer' ? workspaceQuery : '';
        if (workspaceMode === 'viewer') {
            const metadata = await readJson('/state/meta' + query, options);
            if (stopped === true) { return; }
            if (replay.active === true || (savedStateTag !== null && metadata.result.tag === savedStateTag)) {
                setConnection(true);
                element('notice').hidden = graphicsError === null;
                return;
            }
        }

        const request = savedStateTag === null ? options : { ...options, headers: { 'If-None-Match': savedStateTag } };
        const response = await fetch('/state' + query, request);
        if (stopped === true) { return; }
        if (response.status === 304) {
            setConnection(true);
            element('notice').hidden = graphicsError === null;
            return;
        }
        const data = await checkedJson(response);
        if (stopped === true) { return; }

        element('objects-open').disabled = false;
        element('replay-open').disabled = false;
        let restoreView = false;
        if (typeof data.workspace === 'string') {
            if (workspace !== data.workspace) {
                saveWorkspaceView();
                viewer.showAllObjects();
                inspector.resetExpansion();
                geometryKey = null;
                workspace = data.workspace;
                restoreView = true;
            }
            if (workspaceMode !== 'viewer') {
                const path = data.workspace.startsWith('\\\\?\\') ? data.workspace.slice(4) : data.workspace;
                element('workspace-path').textContent = path;
                element('workspace-path').title = path;
            }
        }
        if (replay.active === false) {
            element('model-name').textContent = workspaceMode === 'viewer' ? 'Saved model' : 'Live model';
            showState(data.result);
            if (restoreView === true) {
                restoreWorkspaceView();
                savedViewerSnapshot = viewerSnapshot();
            }
            savedStateTag = response.headers?.get('ETag') ?? null;
        }
        if (graphicsError !== null) { element('notice').textContent = graphicsError; }
        setConnection(true);
        element('notice').hidden = graphicsError === null;
    } catch (error) {
        if (stopped === true) { return; }

        setConnection(false);
        element('notice').hidden = false;
        if (error instanceof TypeError || error.name === 'AbortError') {
            element('notice').textContent = geometryKey === null ? 'Waiting for the mesh server. Retrying automatically…' : 'Connection interrupted. Showing the last model; retrying automatically…';
        } else {
            element('notice').textContent = error.message;
        }
        if (graphicsError !== null) { element('notice').textContent = graphicsError; }
    } finally {
        clearTimeout(timeout);
        controller = null;
        if (stopped === false) { timer = setTimeout(refresh, document.hidden === true ? 2000 : 400); }
    }
}

function initialize() {
    try {
        viewer = new ModelViewer(element('viewport'));
    } catch (error) {
        element('notice').textContent = error.message;
        element('connection').textContent = 'Viewer unavailable';
        document.querySelectorAll('button').forEach(button => { button.disabled = true; });
        return;
    }
    element('replay-open').addEventListener('click', saveWorkspaceView);
    replay = new SessionReplay(showState, () => {
        savedStateTag = null;
    });
    inspector = new ObjectInspector(viewer);
    const updateCameraGizmo = setupCameraGizmo();
    viewer.onAnimationFrame = seconds => {
        updateCameraGizmo();
        advanceAnimation(seconds);
    };
    const clipSearch = element('animation-clip-search');
    const clipPicker = element('animation-clip-picker');
    element('animation-clip-open').addEventListener('click', () => {
        clipSearch.value = '';
        clipPicker.showModal();
        renderClipMatches();
    });
    clipPicker.addEventListener('click', event => { if (event.target === clipPicker) { clipPicker.close(); } });
    clipSearch.addEventListener('input', renderClipMatches);
    clipSearch.addEventListener('keydown', event => {
        if (event.isComposing === true || clipHighlight === -1) { return; }

        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault();
            const step = event.key === 'ArrowDown' ? 1 : -1;
            highlightClip((clipHighlight + step + clipMatches.length) % clipMatches.length);
        } else if (event.key === 'Enter') {
            event.preventDefault();
            chooseClip(clipMatches[clipHighlight].name);
        }
    });
    element('animation-play').addEventListener('click', event => {
        if (animation === null) { return; }

        animation.playing = !animation.playing;
        if (animation.playing === true && animation.time === animation.clip.duration) { animation.time = 0; }
        event.target.textContent = animation.playing ? 'Pause' : 'Play';
    });
    element('animation-time').addEventListener('input', event => {
        seekAnimation(Number(event.target.value));
    });
    for (const name of Object.keys(rigDisplay)) {
        const button = element(`animation-${name}`);
        button.addEventListener('click', () => {
            rigDisplay[name] = !rigDisplay[name];
            button.setAttribute('aria-pressed', String(rigDisplay[name]));
            showAnimationFrame();
        });
    }
    const picker = element('workspace-picker');
    const search = element('workspace-search');
    element('workspace-open').addEventListener('click', openWorkspacePicker);
    picker.addEventListener('click', event => { if (event.target === picker) { picker.close(); } });
    search.addEventListener('input', renderWorkspaceMatches);
    search.addEventListener('keydown', event => {
        if (event.isComposing === true || workspaceSelection === -1) { return; }
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault();
            const step = event.key === 'ArrowDown' ? 1 : -1;
            const count = workspaceMatches.length;
            highlightWorkspace((workspaceSelection + step + count) % count);
        } else if (event.key === 'Enter') {
            event.preventDefault();
            selectWorkspace(workspaceMatches[workspaceSelection]);
        }
    });
    const settings = element('viewer-settings');
    const settingsButton = element('settings-button');
    for (const panel of ['objects', 'replay']) {
        element(`${panel}-open`).addEventListener('click', () => {
            if (panel === 'objects') { replay.hide(); }
            else { inspector.setOpen(false); }

            settings.open = false;
            const focusTarget = element(`${panel}-panel`).hidden ? settingsButton : element(`${panel}-close`);
            focusTarget.focus();
        });
        element(`${panel}-close`).addEventListener('click', () => settingsButton.focus());
    }
    for (const name of ['grid', 'turntable']) {
        element(name).addEventListener('click', () => {
            viewer[name] = viewer[name] === false;
            viewer.dirty = true;
            element(name).setAttribute('aria-pressed', String(viewer[name]));
        });
    }
    for (const button of document.querySelectorAll('[data-viewer-mode]')) {
        button.addEventListener('click', () => selectViewerMode(button.dataset.viewerMode));
    }
    const disclosures = document.querySelectorAll('.toolbar-disclosure');
    for (const button of document.querySelectorAll('[data-render-mode]')) {
        button.addEventListener('click', () => {
            selectRenderMode(button.dataset.renderMode);
            const debug = button.closest('details');
            if (debug === null) { return; }

            debug.open = false;
            debug.querySelector('summary').focus();
        });
    }
    document.addEventListener('click', event => {
        for (const disclosure of disclosures) {
            if (disclosure.contains(event.target) === false) { disclosure.open = false; }
        }
    });
    document.addEventListener('keydown', event => {
        if (workspaceMode === 'viewer' && event.key.toLowerCase() === 'p' && event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey) {
            event.preventDefault();
            openWorkspacePicker();
            return;
        }
        if (event.key === 'Escape' && picker.open === false) {
            for (const disclosure of disclosures) {
                if (disclosure.open === false) { continue; }

                disclosure.open = false;
                if (disclosure.contains(event.target)) { disclosure.querySelector('summary').focus(); }
            }
        }
    });
    element('viewport').addEventListener('viewer-error', event => {
        graphicsError = event.detail;
        element('notice').hidden = false;
        element('notice').textContent = graphicsError;
    });
    element('viewport').addEventListener('viewer-restored', () => {
        showAnimationFrame();
        graphicsError = null;
        element('notice').hidden = true;
    });
    document.addEventListener('visibilitychange', () => {
        viewer.lastTime = null;
        if (document.hidden === true) { saveWorkspaceView(); }
    });
    window.addEventListener('pagehide', () => {
        saveWorkspaceView();
        stopped = true;
        clearTimeout(timer);
        controller?.abort();
        replay.dispose();
        inspector.dispose();
        viewer.dispose();
    });
    window.addEventListener('pageshow', event => { if (event.persisted === true) { window.location.reload(); } });
    refresh();
}

initialize();
