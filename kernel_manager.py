"""Runs one kernel process per notebook connection and relays messages over a WebSocket."""
import asyncio
import json
import os
import signal
import subprocess
import sys
import threading
from collections import deque

from fastapi import WebSocket, WebSocketDisconnect

from arabic_translator import translate_to_python

KERNEL_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'kernel.py')
INTERRUPT_GRACE_SECONDS = 2
MAX_STREAM_BATCH = 64 * 1024


class Kernel:
    """A kernel subprocess whose output lines are put on `queue` as (generation, message)."""

    def __init__(self, loop, queue, generation):
        self.loop = loop
        self.queue = queue
        self.generation = generation
        flags = subprocess.CREATE_NEW_PROCESS_GROUP if os.name == 'nt' else 0
        self.proc = subprocess.Popen(
            [sys.executable, '-I', '-X', 'utf8', '-u', KERNEL_PATH],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding='utf-8',
            bufsize=1,
            creationflags=flags,
            start_new_session=os.name != 'nt',
        )
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            self.loop.call_soon_threadsafe(self.queue.put_nowait, (self.generation, msg))
        self.loop.call_soon_threadsafe(self.queue.put_nowait, (self.generation, {'type': 'exit'}))

    def send(self, msg):
        try:
            self.proc.stdin.write(json.dumps(msg, ensure_ascii=False) + '\n')
            self.proc.stdin.flush()
        except (BrokenPipeError, OSError):
            pass

    def interrupt(self):
        try:
            if os.name == 'nt':
                self.proc.send_signal(signal.CTRL_BREAK_EVENT)
            else:
                self.proc.send_signal(signal.SIGINT)
        except OSError:
            pass

    def kill(self):
        if self.proc.poll() is None:
            self.proc.kill()


class NotebookSession:
    """Queues cell executions and forwards kernel output to the browser, tagged by cell id."""

    def __init__(self, ws: WebSocket):
        self.ws = ws
        self.loop = asyncio.get_running_loop()
        self.queue = asyncio.Queue()  # (generation, message) from every kernel this session started
        self.generation = 0
        self.kernel = None
        self.pending = deque()        # (cell_id, code) waiting to run
        self.running = None           # cell id currently executing
        self.ready = False
        self.waiting_input = False
        self.interrupt_timer = None

    def start_kernel(self):
        self.generation += 1
        self.ready = False
        self.kernel = Kernel(self.loop, self.queue, self.generation)

    async def restart(self, reason):
        self._cancel_interrupt_timer()
        self.kernel.kill()
        # Everything queued or running is cancelled
        cancelled = ([self.running] if self.running else []) + [cid for cid, _ in self.pending]
        self.pending.clear()
        self.running = None
        self.waiting_input = False
        for cid in cancelled:
            await self.ws.send_json({'type': 'done', 'cell_id': cid, 'cancelled': True})
        await self.ws.send_json({'type': 'kernel', 'state': 'restarting', 'reason': reason})
        self.start_kernel()

    async def run_next(self):
        if not self.ready or self.running or not self.pending:
            return
        cell_id, code = self.pending.popleft()
        self.running = cell_id
        translated = translate_to_python(code.replace('\r\n', '\n'))
        await self.ws.send_json({'type': 'started', 'cell_id': cell_id, 'translated': translated})
        await self.ws.send_json({'type': 'kernel', 'state': 'busy'})
        self.kernel.send({'code': translated})

    def _cancel_interrupt_timer(self):
        if self.interrupt_timer:
            self.interrupt_timer.cancel()
            self.interrupt_timer = None

    async def handle_client(self):
        while True:
            msg = await self.ws.receive_json()
            kind = msg.get('type')
            if kind == 'execute':
                self.pending.append((msg['cell_id'], msg.get('code', '')))
                await self.ws.send_json({'type': 'queued', 'cell_id': msg['cell_id']})
                await self.run_next()
            elif kind == 'input_reply' and self.waiting_input:
                self.waiting_input = False
                self.kernel.send({'input': msg.get('value', '')})
            elif kind == 'interrupt':
                cancelled = [cid for cid, _ in self.pending]
                self.pending.clear()
                for cid in cancelled:
                    await self.ws.send_json({'type': 'done', 'cell_id': cid, 'cancelled': True})
                if not self.running or self.interrupt_timer:
                    continue
                if self.waiting_input:
                    self.waiting_input = False
                    self.kernel.send({'interrupt': True})
                else:
                    self.kernel.interrupt()
                # If the cell doesn't stop in time (e.g. stuck in C code), restart the kernel
                self.interrupt_timer = self.loop.call_later(
                    INTERRUPT_GRACE_SECONDS,
                    lambda: asyncio.ensure_future(self.restart('interrupt_timeout')),
                )
            elif kind == 'restart':
                await self.restart('user')

    async def handle_kernel(self):
        while True:
            generation, msg = await self.queue.get()
            if generation != self.generation:
                continue  # output from a kernel that has been replaced
            kind = msg.get('type')

            if kind == 'ready':
                self.ready = True
                await self.ws.send_json({'type': 'kernel', 'state': 'idle'})
                await self.run_next()
                continue
            if kind == 'exit':
                # Kernel died on its own (e.g. os._exit); restart it
                if self.running:
                    await self.ws.send_json({'type': 'error', 'cell_id': self.running,
                                             'text': 'KernelDied', 'line': None})
                await self.restart('crash')
                continue

            if kind == 'stream':
                # Merge bursts of output into one message to avoid flooding the browser
                text = msg['text']
                while len(text) < MAX_STREAM_BATCH and not self.queue.empty():
                    gen, nxt = self.queue._queue[0]
                    if gen != generation or nxt.get('type') != 'stream' or nxt.get('name') != msg.get('name'):
                        break
                    text += self.queue.get_nowait()[1]['text']
                msg['text'] = text
            elif kind == 'input_request':
                self.waiting_input = True
            elif kind == 'done':
                self._cancel_interrupt_timer()

            msg['cell_id'] = self.running
            await self.ws.send_json(msg)

            if kind == 'done':
                self.running = None
                if not self.pending:
                    await self.ws.send_json({'type': 'kernel', 'state': 'idle'})
                await self.run_next()

    async def serve(self):
        self.start_kernel()
        kernel_task = asyncio.create_task(self.handle_kernel())
        try:
            await self.handle_client()
        except WebSocketDisconnect:
            pass
        finally:
            kernel_task.cancel()
            self._cancel_interrupt_timer()
            self.kernel.kill()
