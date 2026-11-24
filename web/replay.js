const element = id => document.getElementById(id);

export class SessionReplay {
    constructor(showState, onLive) {
        this.showState = showState;
        this.onLive = onLive;
        this.workspaceQuery = '';
        this.active = false;
        this.events = [];
        this.position = 0;
        this.session = null;
        this.timer = null;
        this.generation = 0;
        this.disposed = false;
        this.detail = null;
        this.detailMode = 'input';
        for (const mode of ['input', 'output', 'full']) {
            element(`replay-${mode}-tab`).addEventListener('click', () => {
                this.detailMode = mode;
                this.renderDetail();
            });
        }
        element('replay-open').addEventListener('click', () => {
            if (element('replay-panel').hidden === false) { this.close(); }
            else { this.open(); }
        });
        element('replay-close').addEventListener('click', () => this.close());
        element('replay-live').addEventListener('click', () => this.live());
        element('return-to-latest').addEventListener('click', () => this.live());
        element('session-list').addEventListener('change', event => this.load(event.target.value));
        element('replay-position').addEventListener('input', event => { this.pause(); this.seek(Number(event.target.value)); });
        element('replay-previous').addEventListener('click', () => { this.pause(); this.seek(this.position - 1); });
        element('replay-next').addEventListener('click', () => { this.pause(); this.seek(this.position + 1); });
        element('replay-play').addEventListener('click', () => this.play());
        element('replay-search').addEventListener('input', () => this.renderEvents());
    }

    async json(path) {
        const response = await fetch(path + this.workspaceQuery, { cache: 'no-store' });
        const data = await response.json();
        if (response.ok === false || data.error) { throw new Error(data.error || 'Unable to read session.'); }
        return data;
    }

    error(error) {
        if (this.session !== null) { element('session-list').value = this.session; }
        element('replay-summary').textContent = error.message;
        this.pause();
    }

    async open() {
        const generation = ++this.generation;
        element('replay-panel').hidden = false;
        element('replay-open').setAttribute('aria-expanded', 'true');
        try {
            const data = await this.json('/sessions');
            if (this.disposed || generation !== this.generation) { return; }
            element('session-list').replaceChildren(...[...(data.timeline ? [data.timeline] : []), ...data.sessions].map(id => {
                const option = document.createElement('option');
                option.value = id;
                option.textContent = id === 'workspace' ? 'Entire workspace · All sessions' : `${id === data.current ? 'Current · ' : ''}${new Date(Number(id.split('-')[0]) / 1e6).toLocaleString()}`;
                return option;
            }));
            const session = data.timeline ?? data.preferred ?? data.current;
            if (session === null || session === undefined) {
                element('replay-summary').textContent = 'No recorded sessions.';
                return;
            }
            await this.load(session);
        } catch (error) { if (generation === this.generation && !this.disposed) { this.error(error); } }
    }

    async load(id) {
        this.pause();
        const generation = ++this.generation;
        try {
            const data = await this.json(`/sessions/${id}`);
            if (generation !== this.generation || this.disposed) { return; }
            if (!Array.isArray(data.events)) { throw new Error('Invalid session index.'); }
            await this.seek(0, { session: id, initialSession: data.initial_session ?? id, events: data.events });
        } catch (error) { if (generation === this.generation && !this.disposed) { this.error(error); } }
    }

    renderEvents() {
        const query = element('replay-search').value.toLowerCase();
        const buttons = this.events.flatMap((event, index) => {
            if (!JSON.stringify(event).toLowerCase().includes(query)) { return []; }
            const position = index + 1;
            const button = document.createElement('button');
            button.type = 'button';
            button.textContent = `${event.id}. ${event.command || 'Invalid request'}${event.ok ? '' : ' · FAILED'} · ${event.duration_ms.toFixed(1)} ms · ${event.response_bytes} B`;
            button.setAttribute('aria-pressed', String(position === this.position));
            button.addEventListener('click', () => { this.pause(); this.seek(position); });
            return button;
        });
        element('replay-events').replaceChildren(...buttons);
    }

    renderDetail() {
        for (const mode of ['input', 'output', 'full']) {
            element(`replay-${mode}-tab`).setAttribute('aria-pressed', String(mode === this.detailMode));
        }
        const field = this.detailMode === 'full' ? 'full_output' : this.detailMode;
        const value = this.detail?.[field] ?? { note: 'Initial session state. Select a command to inspect its input and output.' };
        element('replay-detail').textContent = JSON.stringify(value, (key, value) => typeof value === 'string' && value.length > 10000 ? `[${value.length} characters; download the event for the full value]` : value, 2);
        element('replay-detail').scrollTop = 0;
    }

    async seek(position, recording = this) {
        const { session, initialSession, events } = recording;
        if (session === null || position < 0 || position > events.length) { return; }

        const generation = ++this.generation;
        const event = events[position - 1];
        const sourceSession = event?.session ?? initialSession ?? session;
        const sourceEvent = event?.source_event ?? event?.id ?? position;
        try {
            const [state, detail] = await Promise.all([
                this.json(`/sessions/${sourceSession}/states/${event?.snapshot ?? 0}`),
                position === 0 ? Promise.resolve({ note: 'Initial session state. Replay never changes the live document.' }) : this.json(`/sessions/${sourceSession}/events/${sourceEvent}`)
            ]);
            if (generation !== this.generation || this.disposed) { return; }
            this.showState(state);
            if (recording !== this) {
                this.session = session;
                this.initialSession = initialSession;
                this.events = events;
                element('session-list').value = session;
                element('replay-position').max = events.length;
                const bytes = events.reduce((sum, event) => sum + event.response_bytes, 0);
                const duration = events.reduce((sum, event) => sum + event.duration_ms, 0);
                element('replay-summary').textContent = `${events.length} commands · ${(bytes / 1024).toFixed(1)} KiB returned · ${duration.toFixed(1)} ms tool execution`;
            }

            this.active = true;
            element('return-to-latest').hidden = false;
            this.position = position;
            element('replay-position').value = position;
            element('model-name').textContent = `Replay · ${position}/${this.events.length}${event ? ` · ${event.command}` : ' · Initial state'}`;
            this.detail = detail;
            this.renderDetail();
            const capture = detail.full_output?.result?.png_base64;
            element('replay-capture').hidden = !capture;
            element('replay-capture').src = capture ? `data:image/png;base64,${capture}` : '';
            const recordPath = position === 0 ? `/sessions/${sourceSession}/states/0` : `/sessions/${sourceSession}/events/${sourceEvent}`;
            element('replay-download').href = recordPath + this.workspaceQuery;
            this.renderEvents();
        } catch (error) { if (generation === this.generation) { this.error(error); } }
    }

    play() {
        if (this.timer !== null) { this.pause(); return; }
        if (this.position >= this.events.length) { this.seek(0); }
        element('replay-play').textContent = 'Pause';
        const advance = async () => {
            if (this.position >= this.events.length) { this.pause(); return; }
            await this.seek(this.position + 1);
            if (this.timer !== null && this.disposed === false) { this.timer = setTimeout(advance, 700); }
        };
        this.timer = setTimeout(advance, 700);
    }

    pause() {
        clearTimeout(this.timer);
        this.timer = null;
        element('replay-play').textContent = 'Play';
    }

    live() {
        this.pause();
        this.generation += 1;
        this.active = false;
        element('return-to-latest').hidden = true;
        this.onLive();
    }

    close() {
        this.live();
        this.hide();
    }

    hide() {
        this.pause();
        this.generation += 1;
        element('replay-panel').hidden = true;
        element('replay-open').setAttribute('aria-expanded', 'false');
    }

    dispose() {
        this.disposed = true;
        this.generation += 1;
        this.pause();
    }
}
