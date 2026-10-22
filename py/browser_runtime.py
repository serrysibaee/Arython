"""Runs Arython code inside Pyodide for the GitHub Pages build.

Loaded by static/pyworker.js, which provides the `arython_host` module:
  send(json)          post a message to the page
  read_input(prompt)  block until the page answers an input request; returns JSON
"""
import builtins
import io
import json
import sys
import time

import arython_host as host
import kernel
from arabic_translator import TIMEOUT_SECONDS, translate_to_python

FLUSH_CHARS = 8192
FLUSH_SECONDS = 0.05

_original_input = builtins.input
_namespace = {'__name__': '__main__', '__builtins__': builtins}
_pending = []  # [stream name, text] chunks not yet sent
_pending_size = 0
_last_flush = time.monotonic()


def flush_streams():
    global _pending_size, _last_flush
    for name, text in _pending:
        host.send(json.dumps({'type': 'stream', 'name': name, 'text': text}, ensure_ascii=False))
    _pending.clear()
    _pending_size = 0
    _last_flush = time.monotonic()


def send(msg):
    flush_streams()  # keep printed output ahead of results and errors
    host.send(json.dumps(msg, ensure_ascii=False))


# kernel.run_cell reports results and errors through kernel.send
kernel.send = send


class StreamWriter:
    """Batches printed text so tight print loops don't flood the page with messages."""

    def __init__(self, name):
        self.name = name

    def write(self, text):
        global _pending_size
        if not text:
            return 0
        if _pending and _pending[-1][0] == self.name:
            _pending[-1][1] += text
        else:
            _pending.append([self.name, text])
        _pending_size += len(text)
        if _pending_size >= FLUSH_CHARS or time.monotonic() - _last_flush >= FLUSH_SECONDS:
            flush_streams()
        return len(text)

    def flush(self):
        pass


_stdout = StreamWriter('stdout')
_stderr = StreamWriter('stderr')


def kernel_input(prompt=''):
    prompt = str(prompt)
    send({'type': 'input_request', 'prompt': prompt})
    reply = json.loads(host.read_input(prompt))
    if reply.get('unsupported'):
        raise RuntimeError('الإدخال التفاعلي غير مدعوم في هذا المتصفح')
    if reply.get('interrupt'):
        raise KeyboardInterrupt
    value = reply['value']
    # Echo like a terminal would
    _stdout.write(f'{prompt}{value}\n')
    return value


def run_cell(arabic_code):
    """Notebook: run one cell in the shared namespace."""
    translated = translate_to_python(arabic_code.replace('\r\n', '\n'))
    send({'type': 'started', 'translated': translated})
    saved = sys.stdout, sys.stderr, builtins.input
    sys.stdout, sys.stderr, builtins.input = _stdout, _stderr, kernel_input
    try:
        kernel.run_cell(translated, _namespace)
    finally:
        flush_streams()
        sys.stdout, sys.stderr, builtins.input = saved
    send({'type': 'done'})


def run_script(arabic_code, stdin_text):
    """Editor: run a whole program in a fresh namespace, like run_arabic_code on the server."""
    python_code = translate_to_python(arabic_code.replace('\r\n', '\n'))
    send({'type': 'script_started', 'translated': python_code})
    out = io.StringIO()
    saved = sys.stdout, sys.stderr, sys.stdin, builtins.input
    sys.stdout, sys.stderr, sys.stdin = out, io.StringIO(), io.StringIO(stdin_text)
    builtins.input = _original_input  # reads sys.stdin
    error = None
    try:
        namespace = {'__name__': '__main__', '__builtins__': builtins}
        exec(compile(python_code, kernel.CELL_FILENAME, 'exec'), namespace)
    except SystemExit as exc:
        if exc.code not in (None, 0):
            error = (f'SystemExit: {exc.code}', None)
    except KeyboardInterrupt:
        return json.dumps({'translated': python_code, 'error': True,
                           'output': f'خطأ: تجاوز البرنامج الحد الزمني ({TIMEOUT_SECONDS} ثوانٍ).'})
    except BaseException as exc:
        error = kernel.format_error(exc)
    finally:
        sys.stdout, sys.stderr, sys.stdin, builtins.input = saved

    output = out.getvalue()
    if error:
        text, line = error
        output += f'خطأ في التنفيذ: {text}' + (f'\n(line {line})' if line else '')
    return json.dumps({'translated': python_code, 'output': output, 'error': error is not None},
                      ensure_ascii=False)


def translate(codes_json):
    return json.dumps([translate_to_python(c.replace('\r\n', '\n')) for c in json.loads(codes_json)],
                      ensure_ascii=False)
