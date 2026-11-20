import { execFile } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { LmmError, requireValue } from './protocol.js';
/** Windows has SID/DACL security, not POSIX uid/mode bits. Never repair an existing ACL. */
export async function windowsJournal(directory, name, action, content = null) {
    requireValue(process.platform === 'win32', 'Windows journal is unavailable.');
    // The package contains the Windows-built helper; no runtime compiler or shell.
    const executable = fileURLToPath(new URL('../src/windows-journal/lmm-magpie-journal.exe', import.meta.url));
    const payload = JSON.stringify({ directory, name, action, content });
    requireValue(Buffer.byteLength(payload) <= 16384, 'Windows journal input is invalid.');
    const unavailable = () => new LmmError('invalid_response', 'Magpie refresh storage could not be verified on Windows.');
    return new Promise((resolve, reject) => {
        let inputFailed = false;
        const started = performance.now();
        const child = execFile(executable, [], {
            windowsHide: true, timeout: 5000, maxBuffer: 4096, encoding: 'utf8',
        }, (error, stdout) => {
            if (error || inputFailed) {
                // Node exposes killed/signal/code, not a distinct timeout cause flag.
                // Elapsed time is an observation; it never changes the original 5s budget.
                const elapsed = Math.max(0, Math.floor(performance.now() - started));
                const code = error?.code;
                const signal = error?.signal;
                try {
                    console.error('lmm_windows_journal_process_failure', JSON.stringify({
                        operation: action === 'pending' || action === 'begin' ? action : 'other',
                        code: typeof code === 'number' && Number.isSafeInteger(code) ? code
                            : typeof code === 'string' && ['ENOENT', 'EACCES', 'EPERM', 'EPIPE', 'ETIMEDOUT', 'ABORT_ERR', 'ERR_CHILD_PROCESS_STDIO_MAXBUFFER'].includes(code) ? code
                                : code == null ? null : 'other',
                        signal: signal == null ? null : ['SIGTERM', 'SIGKILL', 'SIGINT', 'SIGABRT'].includes(signal) ? signal : 'other',
                        killed: error?.killed === true,
                        input_failed: inputFailed,
                        timeout_ms: 5000,
                        elapsed_ms: elapsed,
                        deadline_elapsed: elapsed >= 5000,
                    }));
                }
                catch { /* Diagnostics never replace the original rejection. */ }
                let detail = '';
                try {
                    const failure = JSON.parse(stdout);
                    if (failure?.error === 'unverified_windows_journal'
                        && ['input', 'compile', 'storage', 'result'].includes(failure.stage)
                        && Number.isInteger(failure.nativeCode) && failure.nativeCode >= 0 && failure.nativeCode <= 65535) {
                        detail = ' (' + failure.stage + ', native ' + failure.nativeCode + ')';
                    }
                }
                catch { /* Keep failures fixed and credential-free. */ }
                reject(new LmmError('invalid_response', 'Magpie refresh storage could not be verified on Windows.' + detail));
                return;
            }
            try {
                const result = JSON.parse(stdout);
                if (!result || typeof result !== 'object' || Array.isArray(result))
                    throw unavailable();
                const value = result;
                if (Object.keys(value).sort().join(',') !== 'content,exists'
                    || typeof value.exists !== 'boolean'
                    || (value.content !== null && typeof value.content !== 'string')
                    || (action === 'begin' && (!value.exists || typeof value.content !== 'string'))
                    || (typeof value.content === 'string' && Buffer.byteLength(value.content) >= 256))
                    throw unavailable();
                resolve(value);
            }
            catch {
                reject(unavailable());
            }
        });
        if (!child.stdin) {
            inputFailed = true;
            child.kill();
            return;
        }
        child.stdin.on('error', () => { inputFailed = true; child.kill(); });
        child.stdin.end(payload);
    });
}
