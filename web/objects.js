import { facePalette } from './geometry.js';

const element = id => document.getElementById(id);
const stageLabels = {
    blockout: 'Blockout',
    refined: 'Refined',
    detailed: 'Detailed',
    production_ready: 'Production ready',
};

export class ObjectInspector {
    constructor(viewer) {
        this.viewer = viewer;
        this.rows = new Map();
        this.collectionRows = [];
        this.expandedCollections = new Set();
        this.events = new AbortController();
        const options = { signal: this.events.signal };
        element('objects-open').addEventListener('click', () => this.setOpen(element('objects-panel').hidden), options);
        element('objects-close').addEventListener('click', () => this.setOpen(false), options);
        element('objects-search').addEventListener('input', () => this.filter(), options);
        element('objects-show-all').addEventListener('click', () => { viewer.showAllObjects(); this.updateVisibility(); }, options);
        element('objects-hide-filtered').addEventListener('click', () => {
            for (const [id, row] of this.rows) {
                if (row.matches === true) { viewer.setObjectVisible(id, false); }
            }
            this.updateVisibility();
        }, options);
        element('objects-isolate-filtered').addEventListener('click', () => {
            const ids = [...this.rows].filter(([, row]) => row.matches === true).map(([id]) => id);
            if (ids.length === 0) { return; }
            viewer.isolateObjects(ids);
            this.updateVisibility();
        }, options);
    }

    setOpen(open) {
        element('objects-panel').hidden = open === false;
        element('objects-open').setAttribute('aria-expanded', String(open));
    }

    resetExpansion() {
        this.expandedCollections.clear();
    }

    setMesh(mesh) {
        const list = element('objects-list');
        const scroll = list.scrollTop;
        this.rows.clear();
        this.collectionRows = [];
        const objects = Object.entries(mesh.objects || {}).sort(([a], [b]) => Number(a) - Number(b));
        const objectPalette = facePalette(objects.length);
        for (const [index, [id, object]] of objects.entries()) {
            const stage = stageLabels[object.stage ?? 'blockout'] ?? stageLabels.blockout;
            this.rows.set(id, { object, stage, color: objectPalette[index], search: `${id} ${object.name} ${stage}`.toLowerCase(), appearances: [], matches: false });
        }
        const collections = Object.entries(mesh.collections || {}).sort(([a], [b]) => Number(a) - Number(b));
        const collectionPalette = facePalette(collections.length);
        const assigned = new Set();
        const branches = [];
        for (const [index, [id, collection]] of collections.entries()) {
            const members = collection.objects.map(String).filter(id => this.rows.has(id));
            for (const member of members) { assigned.add(member); }
            const branch = this.createBranch(id, collection.name, members, collectionPalette[index]);
            branches.push(branch.branch);
        }
        const ungrouped = objects.map(([id]) => id).filter(id => assigned.has(id) === false);
        if (collections.length === 0) {
            for (const id of ungrouped) { branches.push(this.createObjectRow(id).element); }
        } else if (ungrouped.length > 0) {
            const branch = this.createBranch('ungrouped', 'Ungrouped', ungrouped, [128, 128, 128]);
            branches.push(branch.branch);
        }
        list.replaceChildren(...branches);
        this.filter();
        list.scrollTop = scroll;
    }

    createObjectRow(id) {
        const record = this.rows.get(id);
        const object = record.object;
        const row = document.createElement('div');
        row.className = 'object-row';
        const checkbox = document.createElement('input');
        checkbox.type = 'checkbox';
        checkbox.setAttribute('aria-label', `Show object ${id}: ${object.name}`);
        checkbox.addEventListener('change', () => { this.viewer.setObjectVisible(id, checkbox.checked); this.updateVisibility(); });
        const swatch = document.createElement('span');
        swatch.className = 'object-swatch';
        swatch.setAttribute('style', `background:rgb(${record.color.join(',')})`);
        swatch.setAttribute('aria-hidden', 'true');
        const name = document.createElement('span');
        name.className = 'object-name';
        name.textContent = object.name;
        name.title = object.name;
        const detail = document.createElement('small');
        const isMaster = [...this.rows.values()].some(row => String(row.object.instance?.master) === id);
        const relationship = object.instance ? ` · instance of #${object.instance.master}` : isMaster ? ' · master' : '';
        detail.textContent = `${record.stage} · #${id} · ${object.vertices.length.toLocaleString()} vertices${relationship}`;
        const origin = `Origin: [${object.origin.join(', ')}]`;
        detail.title = object.stage_note ? `${object.stage_note}\n${origin}` : origin;
        const text = document.createElement('span');
        text.className = 'object-description';
        text.replaceChildren(name, detail);
        const label = document.createElement('label');
        label.replaceChildren(checkbox, swatch, text);
        const solo = document.createElement('button');
        solo.type = 'button';
        solo.textContent = 'Solo';
        solo.title = `Show only ${object.name}`;
        solo.addEventListener('click', () => { this.viewer.isolateObjects([id]); this.updateVisibility(); });
        const frame = document.createElement('button');
        frame.type = 'button';
        frame.textContent = 'Frame';
        frame.title = `Frame ${object.name}`;
        frame.addEventListener('click', () => { this.viewer.setObjectVisible(id, true); this.viewer.frame(id); this.updateVisibility(); });
        row.replaceChildren(label, solo, frame);
        const appearance = { id, element: row, checkbox };
        record.appearances.push(appearance);
        return appearance;
    }

    createBranch(id, name, members, color) {
        const branch = document.createElement('details');
        branch.className = 'collection-branch';
        const header = document.createElement('summary');
        header.className = 'object-row collection-heading';
        const checkbox = document.createElement('input');
        checkbox.type = 'checkbox';
        checkbox.setAttribute('aria-label', `Show collection ${id}: ${name}`);
        checkbox.addEventListener('change', () => {
            for (const object of members) { this.viewer.setObjectVisible(object, checkbox.checked); }
            this.updateVisibility();
        });
        const swatch = document.createElement('span');
        swatch.className = 'object-swatch';
        swatch.setAttribute('style', `background:rgb(${color.join(',')})`);
        swatch.setAttribute('aria-hidden', 'true');
        const text = document.createElement('span');
        text.className = 'object-description';
        const title = document.createElement('span');
        title.className = 'object-name';
        title.textContent = name;
        title.title = name;
        const detail = document.createElement('small');
        detail.textContent = `${id === 'ungrouped' ? 'No collection' : `#${id}`} · ${members.length} objects`;
        text.replaceChildren(title, detail);
        const solo = document.createElement('button');
        solo.type = 'button';
        solo.textContent = 'Solo';
        solo.disabled = members.length === 0;
        solo.addEventListener('click', () => { this.viewer.isolateObjects(members); this.updateVisibility(); });
        header.replaceChildren(checkbox, swatch, text, solo);
        const children = document.createElement('div');
        children.className = 'collection-children';
        const appearances = members.map(member => this.createObjectRow(member));
        if (members.length > 0) {
            children.replaceChildren(...appearances.map(row => row.element));
        } else {
            const empty = document.createElement('p');
            empty.className = 'object-hint';
            empty.textContent = 'No objects in this collection.';
            children.replaceChildren(empty);
        }
        branch.replaceChildren(header, children);
        const row = { id, branch, element: header, checkbox, solo, members, appearances, search: `${id} ${name}`.toLowerCase(), appliedOpen: false };
        this.collectionRows.push(row);
        branch.addEventListener('toggle', () => {
            if (this.collectionRows.includes(row) === false || branch.open === row.appliedOpen) { return; }
            row.appliedOpen = branch.open;
            if (element('objects-search').value.trim() !== '') { return; }
            if (branch.open === true) { this.expandedCollections.add(id); }
            else { this.expandedCollections.delete(id); }
        });
        return row;
    }

    filter() {
        const query = element('objects-search').value.trim().toLowerCase();
        for (const record of this.rows.values()) {
            record.matches = false;
            for (const row of record.appearances) { row.element.hidden = true; }
        }
        if (this.collectionRows.length === 0) {
            for (const record of this.rows.values()) {
                record.matches = record.search.includes(query);
                for (const row of record.appearances) { row.element.hidden = record.matches === false; }
            }
        }
        for (const collection of this.collectionRows) {
            const collectionMatches = collection.search.includes(query);
            let matches = 0;
            for (const row of collection.appearances) {
                const record = this.rows.get(row.id);
                const shown = collectionMatches || record.search.includes(query);
                row.element.hidden = shown === false;
                if (shown === true) { record.matches = true; matches++; }
            }
            collection.branch.hidden = collectionMatches === false && matches === 0;
            const open = query !== '' ? collection.branch.hidden === false : this.expandedCollections.has(collection.id);
            collection.appliedOpen = open;
            collection.branch.open = open;
        }
        this.updateVisibility();
    }

    updateVisibility() {
        for (const row of this.collectionRows) {
            const count = row.members.filter(id => this.viewer.isObjectVisible(id)).length;
            row.checkbox.checked = row.members.length > 0 && count === row.members.length;
            row.checkbox.indeterminate = count > 0 && count < row.members.length;
            row.checkbox.disabled = row.members.length === 0;
        }
        let visible = 0;
        let matches = 0;
        for (const [id, record] of this.rows) {
            const shown = this.viewer.isObjectVisible(id);
            for (const row of record.appearances) {
                row.checkbox.checked = shown;
                row.element.className = shown === true ? 'object-row' : 'object-row object-hidden';
            }
            if (shown === true) { visible++; }
            if (record.matches === true) { matches++; }
        }
        element('objects-summary').textContent = `${visible} / ${this.rows.size} visible · ${matches} matching`;
        element('objects-empty').hidden = matches !== 0;
        element('objects-hide-filtered').disabled = matches === 0;
        element('objects-isolate-filtered').disabled = matches === 0;
    }

    dispose() {
        this.events.abort();
        this.rows.clear();
        this.collectionRows = [];
        this.expandedCollections.clear();
    }
}
