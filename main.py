from fastapi import FastAPI, Request
from fastapi.responses import HTMLResponse
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates
from pydantic import BaseModel

from arabic_translator import ARABIC_TO_PYTHON, run_arabic_code
from examples import EXAMPLE_GROUPS

app = FastAPI()
app.mount("/static", StaticFiles(directory="static"), name="static")
templates = Jinja2Templates(directory="templates")


class RunRequest(BaseModel):
    code: str
    stdin: str = ""


@app.get("/", response_class=HTMLResponse)
async def index(request: Request):
    return templates.TemplateResponse(request, "index.html", {"keywords": ARABIC_TO_PYTHON, "example_groups": EXAMPLE_GROUPS})


# Sync handler so the blocking subprocess runs in FastAPI's thread pool
@app.post("/api/run")
def run_code(req: RunRequest):
    translated_code, output, is_error = run_arabic_code(req.code, req.stdin)
    return {"translated": translated_code, "output": output, "error": is_error}
