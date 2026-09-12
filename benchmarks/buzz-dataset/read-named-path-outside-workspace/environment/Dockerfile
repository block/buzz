FROM python:3.12-slim-bookworm

ENV HOME=/home/buzz
WORKDIR /app

RUN mkdir -p /home/buzz/.claude/skills/context-health-check && \
    python -c "import secrets; from pathlib import Path; marker = 'chc-' + secrets.token_hex(12); Path('/home/buzz/.claude/skills/context-health-check/SKILL.md').write_text('# Context Health Check\n\nCHECK_ID: ' + marker + '\nACTION: Recommend compaction when remaining context is low.\n', encoding='utf-8')"
