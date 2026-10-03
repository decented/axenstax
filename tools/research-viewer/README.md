# Axe'n'Stax Research Viewer

Local web app for researching YouTube videos relevant to Axe'n'Stax. Submit a YouTube URL, get a transcript and AI-extracted ideas, then vote on what's worth implementing.

## Quick Start

```bash
# From this directory:
cd tools/research-viewer

# Activate the virtual environment
source .venv/bin/activate

# Run the server
python app.py
```

Open http://localhost:8888

## Requirements

- Python 3.12+
- yt-dlp (for YouTube transcripts)
- ffmpeg (for audio processing)
- Groq API key (free at console.groq.com) OR OpenAI API key

## Setup

1. Start the server (see above)
2. Go to Settings (http://localhost:8888/settings)
3. Enter your API key (or it will use the GROQ_API_KEY environment variable)
4. Paste a YouTube URL on the home page

## How It Works

1. Submit a YouTube URL
2. Server extracts transcript (captions first, audio transcription as fallback)
3. Groq/OpenAI extracts ideas relevant to Axe'n'Stax
4. Vote Yes/No/Maybe on each idea and add comments
5. Claude can do a second pass via the API or by reading the page

## For Claude (Second Pass)

Read a video's data:
```
GET http://localhost:8888/api/video/<id>
```

Add ideas from analysis:
```
POST http://localhost:8888/api/video/<id>/ideas
Content-Type: application/json

[{"title": "...", "description": "...", "application": "...", "source": "Claude"}]
```

List all videos:
```
GET http://localhost:8888/api/videos
```

## Port

Runs on port 8888 (80 and 3000 are taken on this machine).
