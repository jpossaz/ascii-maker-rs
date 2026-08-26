// Runs the wasm converter off the main thread and streams finished rows back.
import init, { convert_image, palette_hex } from './out/ascii_maker.js';

let ready = null;

async function ensureReady() {
    if (!ready) {
        ready = init().then(() => {
            self.postMessage({ type: 'palette', palette: palette_hex() });
        });
    }
    return ready;
}

self.onmessage = async (e) => {
    const opts = e.data;
    try {
        await ensureReady();

        const onRow = (row, triples, total) => {
            self.postMessage({ type: 'row', row, total, triples }, [triples.buffer]);
        };

        const art = convert_image(opts.data, opts.options, onRow);

        self.postMessage({
            type: 'done',
            cols: art.cols,
            rows: art.rows,
            text: art.text(),
            ansi: art.ansi(false),
        });
        art.free();
    } catch (error) {
        self.postMessage({ type: 'error', error: String(error.message || error) });
    }
};

self.onerror = (error) => {
    self.postMessage({ type: 'error', error: String(error.message || error) });
};
