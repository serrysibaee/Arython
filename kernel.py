"""Notebook kernel: a long-running Python process that executes cells in one shared namespace.

Protocol (one JSON object per line):
  in : {"code": "<python>"}            run a cell
       {"input": "<text>"}             reply to an input request
  out: {"type": "stream", "text": ...} printed output
       {"type": "input_request", "prompt": ...}
       {"type": "result", "text": ...} value of the cell's last expression
       {"type": "error", "text": ..., "line": n}
       {"type": "done"}
"""
import ast
import builtins
import json
import signal
import sys
import traceback

CELL_FILENAME = '<cell>'

# Keep the real pipes for the protocol; user code only sees the redirected streams
_proto_out = sys.stdout
_proto_in = sys.stdin


def send(msg):
    _proto_out.write(json.dumps(msg, ensure_ascii=False) + '\n')
    _proto_out.flush()


class StreamWriter:
    def __init__(self, name):
        self.name = name

    def write(self, text):
        if text:
            send({'type': 'stream', 'name': self.name, 'text': text})
        return len(text)

    def flush(self):
        pass


def kernel_input(prompt=''):
    prompt = str(prompt)
    send({'type': 'input_request', 'prompt': prompt})
    line = _proto_in.readline()
    if not line:
        raise EOFError
    reply = json.loads(line)
    if reply.get('interrupt'):
        raise KeyboardInterrupt
    value = reply.get('input', '')
    # Echo like a terminal would
    send({'type': 'stream', 'name': 'stdout', 'text': f'{prompt}{value}\n'})
    return value


def format_error(exc):
    """Short message plus the line number inside the cell, if any."""
    if isinstance(exc, SyntaxError) and exc.filename == CELL_FILENAME:
        return f'{type(exc).__name__}: {exc.msg}', exc.lineno
    line = None
    for frame in traceback.extract_tb(exc.__traceback__):
        if frame.filename == CELL_FILENAME:
            line = frame.lineno
    return f'{type(exc).__name__}: {exc}', line


_running = False


def on_interrupt(signum, frame):
    # Only interrupt user code, never the kernel's own message loop
    if _running:
        raise KeyboardInterrupt


def run_cell(code, namespace):
    global _running
    _running = True
    try:
        tree = ast.parse(code, CELL_FILENAME)
        # Like Jupyter: show the value of a trailing expression
        last_expr = None
        if tree.body and isinstance(tree.body[-1], ast.Expr):
            last_expr = ast.Expression(tree.body.pop().value)
        exec(compile(tree, CELL_FILENAME, 'exec'), namespace)
        if last_expr is not None:
            value = eval(compile(last_expr, CELL_FILENAME, 'eval'), namespace)
            if value is not None:
                send({'type': 'result', 'text': repr(value)})
    except KeyboardInterrupt:
        send({'type': 'error', 'text': 'KeyboardInterrupt', 'line': None})
    except BaseException as exc:  # includes SystemExit from exit()
        text, line = format_error(exc)
        send({'type': 'error', 'text': text, 'line': line})
    finally:
        _running = False


def main():
    # The server interrupts with CTRL_BREAK on Windows and SIGINT elsewhere
    signal.signal(getattr(signal, 'SIGBREAK', signal.SIGINT), on_interrupt)
    namespace = {'__name__': '__main__', '__builtins__': builtins}
    builtins.input = kernel_input
    sys.stdout = StreamWriter('stdout')
    sys.stderr = StreamWriter('stderr')
    sys.stdin = None  # input() is the only way to read

    send({'type': 'ready'})
    for line in _proto_in:
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if 'code' in msg:
            run_cell(msg['code'], namespace)
            send({'type': 'done'})


if __name__ == '__main__':
    main()
