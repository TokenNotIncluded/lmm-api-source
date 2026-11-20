import { execFile } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { isAbsolute, join } from 'node:path';
import { LmmError, requireValue } from './protocol.js';
const journalPhases = ['script_enter', 'input_ready', 'compile_begin', 'compile_end', 'storage_begin', 'storage_end', 'result_ready'];
let program;
/** Windows has SID/DACL security, not POSIX uid/mode bits. Never repair an existing ACL. */
export async function windowsJournal(directory, name, action, content = null) {
    requireValue(process.platform === 'win32', 'Windows journal is unavailable.');
    const systemRoot = process.env.SystemRoot;
    requireValue(typeof systemRoot === 'string' && isAbsolute(systemRoot), 'Windows journal is unavailable.');
    const executable = join(systemRoot, 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe');
    // src is part of the existing package file list. This URL resolves identically
    // from the source module and from its compiled dist module.
    program ??= readFile(new URL('../src/magpie-windows-journal.ps1', import.meta.url), 'utf8');
    const fixedProgram = await program;
    const payload = JSON.stringify({ directory, name, action, content });
    requireValue(Buffer.byteLength(payload) <= 16384, 'Windows journal input is invalid.');
    const unavailable = () => new LmmError('invalid_response', 'Magpie refresh storage could not be verified on Windows.');
    return new Promise((resolve, reject) => {
        let inputFailed = false;
        const started = performance.now();
        // Control-only measurement. Never retain or print unrecognized stderr.
        const phaseRecords = [];
        let phasePending = '', phaseInvalidLines = 0, phaseOverflow = false, phaseParserErrors = 0;
        const child = execFile(executable, ['-NoProfile', '-NonInteractive', '-Command', fixedProgram], {
            windowsHide: true, timeout: 5000, maxBuffer: 4096, encoding: 'utf8',
        }, (error, stdout) => {
            try {
                console.error('lmm_windows_journal_phase_measurement', JSON.stringify({
                    operation: action === 'pending' || action === 'begin' ? action : 'other',
                    elapsed_ms: Math.max(0, Math.floor(performance.now() - started)),
                    process_failed: Boolean(error || inputFailed),
                    complete: phaseRecords.length === journalPhases.length,
                    phases: phaseRecords,
                    invalid_lines: phaseInvalidLines,
                    incomplete_line: phasePending.length !== 0,
                    overflow: phaseOverflow,
                    parser_errors: phaseParserErrors,
                }));
            }
            catch { /* Measurement cannot replace an original result or rejection. */ }
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
        child.stderr?.on('data', (chunk) => {
            try {
                const text = typeof chunk === 'string' ? chunk : chunk.toString('utf8');
                if (phasePending.length + text.length > 4096) {
                    phaseOverflow = true;
                    phasePending = '';
                    return;
                }
                phasePending += text;
                let newline;
                while ((newline = phasePending.indexOf('\n')) !== -1) {
                    const line = phasePending.slice(0, newline).replace(/\r$/, '');
                    phasePending = phasePending.slice(newline + 1);
                    const match = /^LMM_JOURNAL_PHASE_V1 (script_enter|input_ready|compile_begin|compile_end|storage_begin|storage_end|result_ready) ([0-9]{1,5})$/.exec(line);
                    const elapsed = match ? Number(match[2]) : -1;
                    if (!match || phaseRecords.length >= journalPhases.length
                        || match[1] !== journalPhases[phaseRecords.length]
                        || !Number.isSafeInteger(elapsed) || elapsed < 0 || elapsed > 60000
                        || elapsed < (phaseRecords.at(-1)?.child_elapsed_ms ?? 0)) {
                        phaseInvalidLines++;
                        continue;
                    }
                    phaseRecords.push({
                        phase: journalPhases[phaseRecords.length],
                        child_elapsed_ms: elapsed,
                        observed_parent_elapsed_ms: Math.max(0, Math.floor(performance.now() - started)),
                    });
                }
            }
            catch {
                phaseParserErrors++;
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
