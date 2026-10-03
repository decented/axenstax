#!/usr/bin/env python3
"""Axe'n'Stax Research Viewer — local web app for YouTube research."""

import json
import os
import re
import subprocess
import threading
import time
import uuid
from datetime import datetime
from pathlib import Path

import httpx
from fastapi import FastAPI, Form, Request
from fastapi.responses import HTMLResponse, RedirectResponse
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates

BASE_DIR = Path(__file__).parent
DATA_DIR = BASE_DIR / "data"
DATA_DIR.mkdir(exist_ok=True)

CONTEXT_FILES = [
    Path(__file__).parent.parent.parent / "CLAUDE.md",
    Path(__file__).parent.parent.parent / "docs" / "vision" / "platform-overview.md",
]

app = FastAPI(title="Axe'n'Stax Research Viewer")
app.mount("/static", StaticFiles(directory=str(BASE_DIR / "static")), name="static")
templates = Jinja2Templates(directory=str(BASE_DIR / "templates"))


# --- Data helpers ---

def load_settings():
    path = DATA_DIR / "settings.json"
    if path.exists():
        return json.loads(path.read_text())
    return {"api_provider": "groq", "api_key": ""}


def save_settings(settings):
    (DATA_DIR / "settings.json").write_text(json.dumps(settings, indent=2))


_VIDEO_ID_RE = re.compile(r'^[a-f0-9]{12}$')


def load_video(video_id):
    # Validate video_id format to prevent path traversal
    if not _VIDEO_ID_RE.match(video_id):
        return None
    path = DATA_DIR / f"{video_id}.json"
    if path.exists():
        return json.loads(path.read_text())
    return None


def save_video(video):
    path = DATA_DIR / f"{video['id']}.json"
    path.write_text(json.dumps(video, indent=2))


def list_videos():
    videos = []
    for f in DATA_DIR.glob("*.json"):
        if f.name == "settings.json":
            continue
        videos.append(json.loads(f.read_text()))
    videos.sort(key=lambda v: v.get("created", ""), reverse=True)
    return videos


def extract_video_id(url):
    """Extract YouTube video ID from various URL formats."""
    patterns = [
        r'(?:v=|/v/|youtu\.be/)([a-zA-Z0-9_-]{11})',
        r'(?:embed/)([a-zA-Z0-9_-]{11})',
        r'(?:shorts/)([a-zA-Z0-9_-]{11})',
    ]
    for pattern in patterns:
        match = re.search(pattern, url)
        if match:
            return match.group(1)
    return None


def get_project_context():
    """Load Axe'n'Stax context for Groq analysis."""
    context_parts = []
    for path in CONTEXT_FILES:
        if path.exists():
            content = path.read_text()
            if len(content) > 3000:
                content = content[:3000] + "\n...(truncated)"
            context_parts.append(f"# {path.name}\n{content}")
    return "\n\n---\n\n".join(context_parts)


def get_existing_youtube_ids():
    """Get set of YouTube IDs already in the system."""
    return {v.get("youtube_id") for v in list_videos() if v.get("youtube_id")}


def search_youtube(query, count=10):
    """Search YouTube via yt-dlp and return video URLs/IDs, skipping existing ones."""
    existing = get_existing_youtube_ids()
    results = []
    # Fetch more than needed to account for duplicates
    fetch_count = count + len(existing) + 5

    try:
        result = subprocess.run(
            ["yt-dlp", f"ytsearch{fetch_count}:{query}",
             "--dump-json", "--no-download", "--flat-playlist"],
            capture_output=True, text=True, timeout=30
        )
        if result.returncode != 0:
            return results

        for line in result.stdout.strip().split("\n"):
            if not line.strip():
                continue
            try:
                meta = json.loads(line)
                yt_id = meta.get("id", "")
                if yt_id and yt_id not in existing:
                    results.append({
                        "youtube_id": yt_id,
                        "title": meta.get("title", "Unknown"),
                        "channel": meta.get("channel", meta.get("uploader", "Unknown")),
                        "duration": meta.get("duration", 0),
                    })
                    if len(results) >= count:
                        break
            except json.JSONDecodeError:
                continue
    except Exception:
        pass
    return results


# --- Processing pipeline ---

processing_lock = threading.Lock()
processing_queue = []


def process_video(video):
    """Full pipeline: metadata → transcript → ideas."""
    settings = load_settings()
    api_key = settings.get("api_key", "") or os.environ.get("GROQ_API_KEY", "")
    provider = settings.get("api_provider", "groq")

    if not api_key:
        video["status"] = "error"
        video["error"] = "No API key configured. Go to Settings."
        save_video(video)
        return

    yt_id = video["youtube_id"]
    url = f"https://www.youtube.com/watch?v={yt_id}"

    # Step 1: Metadata
    video["status"] = "fetching metadata"
    save_video(video)
    try:
        result = subprocess.run(
            ["yt-dlp", "--dump-json", "--no-download", url],
            capture_output=True, text=True, timeout=30
        )
        if result.returncode == 0:
            meta = json.loads(result.stdout)
            video["title"] = meta.get("title", "Unknown")
            video["duration"] = meta.get("duration", 0)
            video["channel"] = meta.get("channel", "Unknown")
            save_video(video)
    except Exception as e:
        video["title"] = video.get("title", "Unknown")
        video["error_meta"] = str(e)
        save_video(video)

    # Step 2: Try YouTube captions first
    video["status"] = "transcribing"
    save_video(video)
    transcript = try_youtube_captions(yt_id)

    # Step 3: If no captions, download and transcribe audio
    if not transcript:
        transcript = transcribe_audio(url, api_key, provider)

    if not transcript:
        video["status"] = "error"
        video["error"] = "Could not extract transcript"
        save_video(video)
        return

    video["transcript"] = transcript

    # Step 4: Extract ideas with Groq/OpenAI
    video["status"] = "extracting ideas"
    save_video(video)
    ideas = extract_ideas(transcript, api_key, provider)
    video["ideas"] = ideas
    video["status"] = "done"
    video["completed"] = datetime.now().isoformat()
    save_video(video)


def try_youtube_captions(yt_id):
    """Try to get YouTube auto-captions without downloading audio."""
    url = f"https://www.youtube.com/watch?v={yt_id}"
    tmp_dir = f"/tmp/rv-{yt_id}"
    os.makedirs(tmp_dir, exist_ok=True)

    try:
        # Try manual subs first, then auto
        for sub_flag in ["--write-sub", "--write-auto-sub"]:
            result = subprocess.run(
                ["yt-dlp", sub_flag, "--sub-lang", "en",
                 "--skip-download", "--sub-format", "vtt",
                 "-o", f"{tmp_dir}/%(title)s", url],
                capture_output=True, text=True, timeout=30
            )
            # Find VTT file
            for f in Path(tmp_dir).glob("*.vtt"):
                vtt_text = f.read_text()
                return clean_vtt(vtt_text)
    except Exception:
        pass
    return None


def clean_vtt(vtt_text):
    """Clean VTT subtitle file to plain text."""
    lines = vtt_text.split("\n")
    seen = set()
    cleaned = []
    for line in lines:
        line = line.strip()
        if not line or line.startswith("WEBVTT") or line.startswith("Kind:") or line.startswith("Language:"):
            continue
        if "-->" in line or re.match(r'^\d+$', line):
            continue
        # Strip HTML tags
        line = re.sub(r'<[^>]+>', '', line)
        if line and line not in seen:
            seen.add(line)
            cleaned.append(line)
    return " ".join(cleaned)


def transcribe_audio(url, api_key, provider):
    """Download audio, compress, chunk, transcribe via API."""
    tmp_dir = f"/tmp/rv-audio-{uuid.uuid4().hex[:8]}"
    os.makedirs(tmp_dir, exist_ok=True)

    try:
        # Download audio
        subprocess.run(
            ["yt-dlp", "-x", "--audio-format", "mp3", "--audio-quality", "5",
             "-o", f"{tmp_dir}/audio.%(ext)s", url],
            capture_output=True, text=True, timeout=300
        )

        audio_file = f"{tmp_dir}/audio.mp3"
        if not os.path.exists(audio_file):
            return None

        # Compress and split into 10-min chunks
        chunk_dir = f"{tmp_dir}/chunks"
        os.makedirs(chunk_dir, exist_ok=True)
        subprocess.run(
            ["ffmpeg", "-i", audio_file, "-ac", "1", "-ar", "16000", "-b:a", "32k",
             "-f", "segment", "-segment_time", "600", f"{chunk_dir}/chunk_%03d.mp3"],
            capture_output=True, text=True, timeout=300
        )

        # Transcribe each chunk
        transcript_parts = []
        chunks = sorted(Path(chunk_dir).glob("chunk_*.mp3"))

        if provider == "groq":
            api_url = "https://api.groq.com/openai/v1/audio/transcriptions"
            model = "whisper-large-v3"
        else:
            api_url = "https://api.openai.com/v1/audio/transcriptions"
            model = "whisper-1"

        for chunk in chunks:
            with open(chunk, "rb") as f:
                response = httpx.post(
                    api_url,
                    headers={"Authorization": f"Bearer {api_key}"},
                    files={"file": (chunk.name, f, "audio/mpeg")},
                    data={"model": model, "language": "en", "response_format": "text"},
                    timeout=120
                )
            if response.status_code == 200:
                transcript_parts.append(response.text.strip())
            time.sleep(2)  # Rate limit buffer

        return " ".join(transcript_parts) if transcript_parts else None

    except Exception:
        return None


def extract_ideas(transcript, api_key, provider):
    """Use Groq/OpenAI to extract Axe'n'Stax-relevant ideas."""
    context = get_project_context()

    prompt = f"""You are analysing a video transcript for ideas relevant to Axe'n'Stax — an open-source voxel sandbox game (like Minecraft) where players earn real Bitcoin by mining blocks.

PROJECT CONTEXT:
{context}

TRANSCRIPT:
{transcript[:12000]}

Extract ALL ideas from this transcript that could be relevant to Axe'n'Stax. For each idea:
1. Give it a short title
2. Explain the idea in 1-2 sentences
3. Explain how it could apply to Axe'n'Stax in 1-2 sentences

Respond in JSON format:
[
  {{"title": "...", "description": "...", "application": "..."}},
  ...
]

Only include genuinely relevant ideas. If nothing is relevant, return an empty array []."""

    if provider == "groq":
        api_url = "https://api.groq.com/openai/v1/chat/completions"
        model = "llama-3.3-70b-versatile"
    else:
        api_url = "https://api.openai.com/v1/chat/completions"
        model = "gpt-4o-mini"

    try:
        response = httpx.post(
            api_url,
            headers={
                "Authorization": f"Bearer {api_key}",
                "Content-Type": "application/json",
            },
            json={
                "model": model,
                "messages": [{"role": "user", "content": prompt}],
                "temperature": 0.3,
            },
            timeout=120,
        )
        if response.status_code == 200:
            content = response.json()["choices"][0]["message"]["content"]
            # Extract JSON from response (might be wrapped in markdown)
            json_match = re.search(r'\[.*\]', content, re.DOTALL)
            if json_match:
                ideas_raw = json.loads(json_match.group())
                return [
                    {
                        "id": uuid.uuid4().hex[:8],
                        "source": "Groq" if provider == "groq" else "OpenAI",
                        "title": idea.get("title", ""),
                        "description": idea.get("description", ""),
                        "application": idea.get("application", ""),
                        "vote": None,
                        "comment": "",
                    }
                    for idea in ideas_raw
                ]
    except Exception:
        pass
    return []


def background_worker():
    """Process videos one at a time from the queue."""
    while True:
        videos = list_videos()
        queued = [v for v in videos if v.get("status") == "queued"]
        if queued:
            video = queued[0]  # oldest first
            with processing_lock:
                process_video(video)
        time.sleep(3)


# --- Routes ---

@app.on_event("startup")
def startup():
    thread = threading.Thread(target=background_worker, daemon=True)
    thread.start()


@app.get("/", response_class=HTMLResponse)
async def home(request: Request):
    videos = list_videos()
    return templates.TemplateResponse("home.html", {"request": request, "videos": videos})


@app.post("/search")
async def search_videos(query: str = Form(...)):
    results = search_youtube(query, count=10)
    for r in results:
        video = {
            "id": uuid.uuid4().hex[:12],
            "youtube_id": r["youtube_id"],
            "url": f"https://www.youtube.com/watch?v={r['youtube_id']}",
            "title": r["title"],
            "channel": r.get("channel", ""),
            "duration": r.get("duration", 0),
            "status": "queued",
            "created": datetime.now().isoformat(),
            "transcript": "",
            "ideas": [],
            "manual_analysis": "",
            "search_query": query,
        }
        save_video(video)
    return RedirectResponse("/", status_code=303)


@app.post("/submit")
async def submit_url(url: str = Form(...)):
    yt_id = extract_video_id(url)
    if not yt_id:
        return RedirectResponse("/", status_code=303)

    # Check if already exists
    existing = [v for v in list_videos() if v.get("youtube_id") == yt_id]
    if existing:
        return RedirectResponse(f"/video/{existing[0]['id']}", status_code=303)

    video = {
        "id": uuid.uuid4().hex[:12],
        "youtube_id": yt_id,
        "url": url.strip(),
        "title": "Loading...",
        "status": "queued",
        "created": datetime.now().isoformat(),
        "transcript": "",
        "ideas": [],
        "manual_analysis": "",
    }
    save_video(video)
    return RedirectResponse("/", status_code=303)


@app.get("/video/{video_id}", response_class=HTMLResponse)
async def video_page(request: Request, video_id: str):
    video = load_video(video_id)
    if not video:
        return RedirectResponse("/", status_code=303)
    return templates.TemplateResponse("video.html", {"request": request, "video": video})


@app.post("/video/{video_id}/vote")
async def vote_idea(video_id: str, request: Request):
    video = load_video(video_id)
    if not video:
        return RedirectResponse("/", status_code=303)

    form = await request.form()
    for idea in video.get("ideas", []):
        vote_key = f"vote_{idea['id']}"
        comment_key = f"comment_{idea['id']}"
        if vote_key in form:
            idea["vote"] = form[vote_key]
        if comment_key in form:
            idea["comment"] = form[comment_key]

    # Save manual analysis
    if "manual_analysis" in form:
        video["manual_analysis"] = form["manual_analysis"]

    save_video(video)
    return RedirectResponse(f"/video/{video_id}", status_code=303)


@app.get("/settings", response_class=HTMLResponse)
async def settings_page(request: Request):
    settings = load_settings()
    return templates.TemplateResponse("settings.html", {"request": request, "settings": settings})


@app.post("/settings")
async def save_settings_route(
    api_provider: str = Form(...),
    api_key: str = Form(...),
):
    save_settings({"api_provider": api_provider, "api_key": api_key})
    return RedirectResponse("/settings", status_code=303)


# --- API endpoints for Claude second-pass ---

@app.get("/api/videos")
async def api_list_videos():
    return list_videos()


@app.get("/api/video/{video_id}")
async def api_get_video(video_id: str):
    video = load_video(video_id)
    if not video:
        return {"error": "not found"}
    return video


@app.post("/api/video/{video_id}/ideas")
async def api_add_ideas(video_id: str, request: Request):
    video = load_video(video_id)
    if not video:
        return {"error": "not found"}

    body = await request.json()
    ideas = body if isinstance(body, list) else [body]

    for idea in ideas:
        video["ideas"].append({
            "id": uuid.uuid4().hex[:8],
            "source": idea.get("source", "Claude"),
            "title": idea.get("title", ""),
            "description": idea.get("description", ""),
            "application": idea.get("application", ""),
            "vote": None,
            "comment": "",
        })

    save_video(video)
    return {"status": "ok", "ideas_added": len(ideas)}


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8888)
