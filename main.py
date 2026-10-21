from fastapi import FastAPI, Request, WebSocket
from fastapi.responses import HTMLResponse
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates
from pydantic import BaseModel

from arabic_translator import ARABIC_TO_PYTHON, run_arabic_code, translate_to_python
from examples import EXAMPLE_GROUPS
from kernel_manager import NotebookSession

app = FastAPI()
app.mount("/static", StaticFiles(directory="static"), name="static")
templates = Jinja2Templates(directory="templates")


class RunRequest(BaseModel):
    code: str
    stdin: str = ""


@app.get("/", response_class=HTMLResponse)
async def index(request: Request):
    return templates.TemplateResponse(request, "index.html", {"keywords": ARABIC_TO_PYTHON, "example_groups": EXAMPLE_GROUPS})


@app.get("/notebook", response_class=HTMLResponse)
async def notebook(request: Request):
    return templates.TemplateResponse(request, "notebook.html", {"keywords": ARABIC_TO_PYTHON})


# Each open notebook gets its own kernel, so variables persist between cells
@app.websocket("/ws/notebook")
async def notebook_ws(ws: WebSocket):
    await ws.accept()
    await NotebookSession(ws).serve()


# Sync handler so the blocking subprocess runs in FastAPI's thread pool
@app.post("/api/run")
def run_code(req: RunRequest):
    translated_code, output, is_error = run_arabic_code(req.code, req.stdin)
    return {"translated": translated_code, "output": output, "error": is_error}


class TranslateRequest(BaseModel):
    codes: list[str]


@app.post("/api/translate")
def translate(req: TranslateRequest):
    return {"translated": [translate_to_python(c.replace("\r\n", "\n")) for c in req.codes]}
