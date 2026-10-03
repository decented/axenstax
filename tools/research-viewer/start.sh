#!/bin/bash
# Axe'n'Stax Research Viewer — start script
cd "$(dirname "$0")"
source .venv/bin/activate
python app.py
