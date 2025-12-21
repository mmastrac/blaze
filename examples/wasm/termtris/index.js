const TICK_INTERVAL = 52;
const BUFFER_HIGH_WATER_MARK = 256;

class Termtris {
    _readQueue = [];
    _readBufferSize = 0;
    _readWaker = null;
    _approxTime = 0;
    _nextTime = 0;
    _wasmInstance = null;
    _startTime;
    _missedTicks = 0;

    constructor() {
        this._startTime = Date.now();
        setInterval(() => this._tick(), TICK_INTERVAL);
    }

    _tick() {
        this._approxTime = Date.now() - this._startTime;
        if (this._wasmInstance !== null) {
            if (this._nextTime < this._approxTime) {
                if (this._readBufferSize < BUFFER_HIGH_WATER_MARK) {
                    // We could use the next tick time here but it's fine to just
                    // call the game update more often instead.
                    this._nextTime = this._wasmInstance.exports._update(this._approxTime) + this._approxTime;
                } else {
                    this._missedTicks++;
                }
            }
        }
    }

    async read() {
        if (this._readQueue.length > 0) {
            return this._doSyncRead();
        }
        const readPromise = new Promise((resolve, reject) => this._readWaker = resolve);
        await readPromise;
        this._readWaker = null;
        return this._doSyncRead();
    }

    _doSyncRead() {
        const result = this._readQueue.shift();
        this._readBufferSize -= result.length;
        if (this._missedTicks > 0 && this._readBufferSize < BUFFER_HIGH_WATER_MARK) {
            console.log("restoring tick", this._missedTicks);
            this._missedTicks = 0;
            this._tick();
        }
        return result;
    }

    write(byte) {
        if (this._wasmInstance === null) {
            return;
        }
        this._wasmInstance.exports._game_input(byte);
        this._approxTime = Date.now() - this._startTime;
        this._nextTime = this._wasmInstance.exports._update(this._approxTime) + this._approxTime;
    }

    // Perform input by hand.
    input(byte) {
        if (typeof byte === 'string') {
            byte = byte.charCodeAt(0);
        }
        this.write(byte);
    }

    _setWasm(instance) {
        instance.exports._initialize();
        instance.exports._init_game();
        this._wasmInstance = instance;
    }
}

function loadTermtris() {
    let wasmMemory = [];

    const args = "\0-t\0/dev/stdin\0";
    const env = "TERM=vt420\0\0";
    const termtris = new Termtris();

    const imports = {
        wasi_snapshot_preview1: {
            args_sizes_get: (args_p, args_len_p) => { 
                new DataView(wasmMemory.buffer).setUint32(args_p, 3, true); // argc
                new DataView(wasmMemory.buffer).setUint32(args_len_p, args.length, true); // argv_buf_size
                return 0;
            },
            args_get: (args_p_p, args_p) => {
                let args_index = 0;
                for (let i = 0; i < 3; i++) {
                    new DataView(wasmMemory.buffer).setUint32(args_p_p + i * 4, args_p + args_index, true);
                    args_index = args.indexOf('\0', args_index) + 1;
                }
                new Uint8Array(wasmMemory.buffer, args_p, args.length).set(new TextEncoder().encode(args));
                return 0;
            },
            proc_exit: (code) => { console.log('Process exited with code:', code); },
            clock_time_get: (clock_id, precision, timestamp) => { 
                console.log('clock_time_get called', clock_id, precision, timestamp);
                new DataView(wasmMemory.buffer).setUint32(timestamp, (termtris._approxTime * 1000) / 0x1_0000_0000, true);
                new DataView(wasmMemory.buffer).setUint32(timestamp + 4, termtris._approxTime * 1000, true);
                return 0;
            },
            clock_res_get: () => {},
            fd_close: () => { console.log('fd_close called'); return 0; },
            fd_read: () => {
                console.log('fd_read called');
                return 1;
            },
            fd_write: (fd, iovs, iov_len, nwritten) => {
                //console.log('fd_write called', fd, iovs, iov_len, nwritten);
                let written = 0;
                for (let i = 0; i < iov_len; i++) {
                    const ptr = new DataView(wasmMemory.buffer).getUint32(iovs + i * 8, true);
                    const len = new DataView(wasmMemory.buffer).getUint32(iovs + i * 8 + 4, true);
                    if (len > 0) {
                        const bytes = Uint8Array.from(new Uint8Array(wasmMemory.buffer, ptr, len));
                        console.log(new TextDecoder().decode(bytes));
                        if (fd == 1) {
                            termtris._readQueue.push(bytes);
                            termtris._readBufferSize += bytes.length;
                        }
                        written += len;
                    }
                }

                if (written > 0 && fd == 1) {
                    if (termtris._readWaker !== null) {
                        termtris._readWaker();
                    }
                }

                new DataView(wasmMemory.buffer).setUint32(nwritten, written, true);
                return 0;
            },
            fd_seek: () => { console.log('fd_seek called'); return 0; },
            environ_sizes_get: (env_p, env_len_p) => { 
                new DataView(wasmMemory.buffer).setUint32(env_p, 1, true);
                new DataView(wasmMemory.buffer).setUint32(env_len_p, env.length, true);
                return 0;
            },
            environ_get: (env_p_p, env_p) => {
                let env_index = 0;
                for (let i = 0; i < 1; i++) {
                    new DataView(wasmMemory.buffer).setUint32(env_p_p + i * 4, env_p + env_index, true);
                    env_index = env.indexOf('\0', env_index) + 1;
                }
                new Uint8Array(wasmMemory.buffer, env_p, env.length).set(new TextEncoder().encode(env));
                return 0;
            },
        },
        env: {
            _log: (message) => {
                console.log(message);
            },
        },
    };

    console.log("Starting WASM module load...");
    fetch('termtris.wasm')
        .then(response => response.arrayBuffer())
        .then(buffer => WebAssembly.instantiate(buffer, imports))
        .then(result => {
            console.log('WASM module loaded!', result.instance.exports);
            wasmMemory = result.instance.exports.memory;
            termtris._setWasm(result.instance);
        })
        .catch(error => console.error('Failed to load WASM:', error));

    return termtris;
}

const termtris = loadTermtris();
async function js_read() {
    return await termtris.read();
}
function js_write(byte) {
    termtris.write(byte);
}
